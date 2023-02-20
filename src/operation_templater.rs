// Copyright 2023 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::io;

use itertools::Itertools as _;
use jujutsu_lib::op_store::{OperationId, OperationMetadata};
use jujutsu_lib::operation::Operation;
use jujutsu_lib::repo::ReadonlyRepo;

use crate::formatter::Formatter;
use crate::template_parser::{
    self, CoreTemplatePropertyKind, FunctionCallNode, IntoTemplateProperty, TemplateAliasesMap,
    TemplateLanguage, TemplateParseError, TemplateParseResult,
};
use crate::templater::{
    IntoTemplate, PlainTextFormattedProperty, Template, TemplateProperty, TemplatePropertyFn,
    TimestampRange,
};

struct OperationTemplateLanguage<'b> {
    head_op_id: &'b OperationId,
}

impl TemplateLanguage<'static> for OperationTemplateLanguage<'_> {
    type Context = Operation;
    type Property = OperationTemplatePropertyKind;

    template_parser::impl_core_wrap_property_fns!('static, OperationTemplatePropertyKind::Core);

    fn build_keyword(&self, name: &str, span: pest::Span) -> TemplateParseResult<Self::Property> {
        build_operation_keyword(self, name, span)
    }

    fn build_method(
        &self,
        property: Self::Property,
        function: &FunctionCallNode,
    ) -> TemplateParseResult<Self::Property> {
        match property {
            OperationTemplatePropertyKind::Core(property) => {
                template_parser::build_core_method(self, property, function)
            }
            OperationTemplatePropertyKind::OperationId(property) => {
                build_operation_id_method(self, property, function)
            }
        }
    }
}

impl OperationTemplateLanguage<'_> {
    fn wrap_operation_id(
        &self,
        property: Box<dyn TemplateProperty<Operation, Output = OperationId>>,
    ) -> OperationTemplatePropertyKind {
        OperationTemplatePropertyKind::OperationId(property)
    }
}

enum OperationTemplatePropertyKind {
    Core(CoreTemplatePropertyKind<'static, Operation>),
    OperationId(Box<dyn TemplateProperty<Operation, Output = OperationId>>),
}

impl IntoTemplateProperty<'static, Operation> for OperationTemplatePropertyKind {
    fn try_into_boolean(self) -> Option<Box<dyn TemplateProperty<Operation, Output = bool>>> {
        match self {
            OperationTemplatePropertyKind::Core(property) => property.try_into_boolean(),
            _ => None,
        }
    }

    fn try_into_integer(self) -> Option<Box<dyn TemplateProperty<Operation, Output = i64>>> {
        match self {
            OperationTemplatePropertyKind::Core(property) => property.try_into_integer(),
            _ => None,
        }
    }

    fn into_plain_text(self) -> Box<dyn TemplateProperty<Operation, Output = String>> {
        match self {
            OperationTemplatePropertyKind::Core(property) => property.into_plain_text(),
            _ => Box::new(PlainTextFormattedProperty::new(self.into_template())),
        }
    }
}

impl IntoTemplate<'static, Operation> for OperationTemplatePropertyKind {
    fn into_template(self) -> Box<dyn Template<Operation>> {
        match self {
            OperationTemplatePropertyKind::Core(property) => property.into_template(),
            OperationTemplatePropertyKind::OperationId(property) => property.into_template(),
        }
    }
}

fn build_operation_keyword(
    language: &OperationTemplateLanguage,
    name: &str,
    span: pest::Span,
) -> TemplateParseResult<OperationTemplatePropertyKind> {
    fn wrap_fn<O>(
        f: impl Fn(&Operation) -> O + 'static,
    ) -> Box<dyn TemplateProperty<Operation, Output = O>> {
        Box::new(TemplatePropertyFn(f))
    }
    fn wrap_metadata_fn<O>(
        f: impl Fn(&OperationMetadata) -> O + 'static,
    ) -> Box<dyn TemplateProperty<Operation, Output = O>> {
        wrap_fn(move |op| f(&op.store_operation().metadata))
    }

    let property = match name {
        "current_operation" => {
            let head_op_id = language.head_op_id.clone();
            language.wrap_boolean(wrap_fn(move |op| op.id() == &head_op_id))
        }
        "description" => {
            language.wrap_string(wrap_metadata_fn(|metadata| metadata.description.clone()))
        }
        "id" => language.wrap_operation_id(wrap_fn(|op| op.id().clone())),
        "tags" => language.wrap_string(wrap_metadata_fn(|metadata| {
            // TODO: introduce map type
            metadata
                .tags
                .iter()
                .map(|(key, value)| format!("{key}: {value}"))
                .join("\n")
        })),
        "time" => language.wrap_timestamp_range(wrap_metadata_fn(|metadata| TimestampRange {
            start: metadata.start_time.clone(),
            end: metadata.end_time.clone(),
        })),
        "user" => language.wrap_string(wrap_metadata_fn(|metadata| {
            // TODO: introduce dedicated type and provide accessors?
            format!("{}@{}", metadata.username, metadata.hostname)
        })),
        _ => return Err(TemplateParseError::no_such_keyword(name, span)),
    };
    Ok(property)
}

impl Template<()> for OperationId {
    fn format(&self, _: &(), formatter: &mut dyn Formatter) -> io::Result<()> {
        formatter.write_str(&self.hex())
    }

    fn has_content(&self, _: &()) -> bool {
        !self.as_bytes().is_empty()
    }
}

fn build_operation_id_method(
    language: &OperationTemplateLanguage,
    self_property: impl TemplateProperty<Operation, Output = OperationId> + 'static,
    function: &FunctionCallNode,
) -> TemplateParseResult<OperationTemplatePropertyKind> {
    let property = match function.name {
        "short" => {
            let ([], [len_node]) = template_parser::expect_arguments(function)?;
            let len_property = len_node
                .map(|node| template_parser::expect_integer_expression(language, node))
                .transpose()?;
            language.wrap_string(template_parser::chain_properties(
                (self_property, len_property),
                TemplatePropertyFn(|(id, len): &(OperationId, Option<i64>)| {
                    let mut hex = id.hex();
                    hex.truncate(len.and_then(|l| l.try_into().ok()).unwrap_or(12));
                    hex
                }),
            ))
        }
        _ => return Err(TemplateParseError::no_such_method("OperationId", function)),
    };
    Ok(property)
}

pub fn parse(
    repo: &ReadonlyRepo,
    template_text: &str,
    aliases_map: &TemplateAliasesMap,
) -> TemplateParseResult<Box<dyn Template<Operation>>> {
    let head_op_id = repo.op_id();
    let language = OperationTemplateLanguage { head_op_id };
    let node = template_parser::parse_template(template_text)?;
    let node = template_parser::expand_aliases(node, aliases_map)?;
    let expression = template_parser::build_expression(&language, &node)?;
    Ok(expression.into_template())
}
