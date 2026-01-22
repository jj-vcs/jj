// Copyright 2024 The Jujutsu Authors
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

use std::cmp::Ordering;
use std::collections::HashMap;

use jj_lib::settings::UserSettings;

use crate::template_builder;
use crate::template_builder::BuildContext;
use crate::template_builder::CoreTemplateBuildFnTable;
use crate::template_builder::CoreTemplatePropertyKind;
use crate::template_builder::CoreTemplatePropertyVar;
use crate::template_builder::TemplateLanguage;
use crate::template_parser;
use crate::template_parser::FunctionCallNode;
use crate::template_parser::TemplateDiagnostics;
use crate::template_parser::TemplateParseResult;
use crate::templater::BoxedSerializeProperty;
use crate::templater::BoxedTemplateProperty;
use crate::templater::ListTemplate;
use crate::templater::Template;
use crate::templater::TemplatePropertyExt as _;

/// General-purpose template language for basic value types.
///
/// This template language only supports the core template property types (plus
/// the self type `C`.) The self type `C` is usually a tuple or struct of value
/// types. It's cloned several times internally. Keyword functions need to be
/// registered to extract properties from the self object.
pub struct GenericTemplateLanguage<'a, C> {
    settings: UserSettings,
    build_fn_table: GenericTemplateBuildFnTable<'a, C>,
}

impl<'a, C> GenericTemplateLanguage<'a, C>
where
    C: serde::Serialize + 'a,
{
    /// Sets up environment with no keywords.
    ///
    /// New keyword functions can be registered by `add_keyword()`.
    pub fn new(settings: &UserSettings) -> Self {
        Self::with_keywords(HashMap::new(), settings)
    }

    /// Sets up environment with the given `keywords` table.
    pub fn with_keywords(
        keywords: GenericTemplateBuildKeywordFnMap<'a, C>,
        settings: &UserSettings,
    ) -> Self {
        Self {
            // Clone settings to keep lifetime simple. It's cheap.
            settings: settings.clone(),
            build_fn_table: GenericTemplateBuildFnTable {
                core: CoreTemplateBuildFnTable::builtin(),
                keywords,
            },
        }
    }

    /// Registers new function that translates keyword to property.
    ///
    /// A keyword function returns `Self::Property`, which is basically a
    /// closure tagged by its return type. The inner closure is usually wrapped
    /// by `TemplateFunction`.
    ///
    /// ```ignore
    /// language.add_keyword("name", |self_property| {
    ///     let out_property = self_property.map(|v| v.to_string());
    ///     Ok(out_property.into_dyn_wrapped())
    /// });
    /// ```
    pub fn add_keyword<F>(&mut self, name: &'static str, build: F)
    where
        F: Fn(
                BoxedTemplateProperty<'a, C>,
            ) -> TemplateParseResult<GenericTemplatePropertyKind<'a, C>>
            + 'a,
    {
        self.build_fn_table.keywords.insert(name, Box::new(build));
    }
}

impl<'a, C> TemplateLanguage<'a> for GenericTemplateLanguage<'a, C>
where
    C: serde::Serialize + 'a,
{
    type Property = GenericTemplatePropertyKind<'a, C>;

    fn settings(&self) -> &UserSettings {
        &self.settings
    }

    fn build_function(
        &self,
        diagnostics: &mut TemplateDiagnostics,
        build_ctx: &BuildContext<Self::Property>,
        function: &FunctionCallNode,
    ) -> TemplateParseResult<Self::Property> {
        let table = &self.build_fn_table.core;
        table.build_function(self, diagnostics, build_ctx, function)
    }

    fn build_method(
        &self,
        diagnostics: &mut TemplateDiagnostics,
        build_ctx: &BuildContext<Self::Property>,
        property: Self::Property,
        function: &FunctionCallNode,
    ) -> TemplateParseResult<Self::Property> {
        let type_name = property.type_name();
        match property {
            GenericTemplatePropertyKind::Core(property) => {
                let table = &self.build_fn_table.core;
                table.build_method(self, diagnostics, build_ctx, property, function)
            }
            GenericTemplatePropertyKind::Self_(property) => {
                let table = &self.build_fn_table.keywords;
                let build = template_parser::lookup_method(type_name, table, function)?;
                // For simplicity, only 0-ary method is supported.
                function.expect_no_arguments()?;
                build(property)
            }
        }
    }
}

pub enum GenericTemplatePropertyKind<'a, C> {
    Core(CoreTemplatePropertyKind<'a>),
    Self_(BoxedTemplateProperty<'a, C>),
}

template_builder::impl_core_property_wrappers!(<'a, C> GenericTemplatePropertyKind<'a, C> => Core);

/// Implements conversion trait for the self property type.
///
/// Since we cannot guarantee that the generic type `C` does not conflict with
/// the core template types, the conversion trait has to be implemented for each
/// concrete type.
macro_rules! impl_self_property_wrapper {
    ($context:path) => {
        $crate::template_builder::impl_property_wrappers!(
            $crate::generic_templater::GenericTemplatePropertyKind<'static, $context> {
                Self_($context),
            }
        );
    };
    (<$a:lifetime> $context:path) => {
        $crate::template_builder::impl_property_wrappers!(
            <$a> $crate::generic_templater::GenericTemplatePropertyKind<$a, $context> {
                Self_($context),
            }
        );
    };
}

pub(crate) use impl_self_property_wrapper;

impl<'a, C> CoreTemplatePropertyVar<'a> for GenericTemplatePropertyKind<'a, C>
where
    C: serde::Serialize + 'a,
{
    fn wrap_template(template: Box<dyn Template + 'a>) -> Self {
        Self::Core(CoreTemplatePropertyKind::wrap_template(template))
    }

    fn wrap_list_template(template: Box<dyn ListTemplate + 'a>) -> Self {
        Self::Core(CoreTemplatePropertyKind::wrap_list_template(template))
    }

    fn type_name(&self) -> &'static str {
        match self {
            Self::Core(property) => property.type_name(),
            Self::Self_(_) => "Self",
        }
    }

    fn try_into_boolean(self) -> Option<BoxedTemplateProperty<'a, bool>> {
        match self {
            Self::Core(property) => property.try_into_boolean(),
            Self::Self_(_) => None,
        }
    }

    fn try_into_integer(self) -> Option<BoxedTemplateProperty<'a, i64>> {
        match self {
            Self::Core(property) => property.try_into_integer(),
            Self::Self_(_) => None,
        }
    }

    fn try_into_timestamp(self) -> Option<BoxedTemplateProperty<'a, jj_lib::backend::Timestamp>> {
        match self {
            Self::Core(property) => property.try_into_timestamp(),
            Self::Self_(_) => None,
        }
    }

    fn try_into_stringify(self) -> Option<BoxedTemplateProperty<'a, String>> {
        match self {
            Self::Core(property) => property.try_into_stringify(),
            Self::Self_(_) => None,
        }
    }

    fn try_into_serialize(self) -> Option<BoxedSerializeProperty<'a>> {
        match self {
            Self::Core(property) => property.try_into_serialize(),
            Self::Self_(property) => Some(property.into_serialize()),
        }
    }

    fn try_into_template(self) -> Option<Box<dyn Template + 'a>> {
        match self {
            Self::Core(property) => property.try_into_template(),
            Self::Self_(_) => None,
        }
    }

    fn try_into_eq(self, other: Self) -> Option<BoxedTemplateProperty<'a, bool>> {
        match (self, other) {
            (Self::Core(lhs), Self::Core(rhs)) => lhs.try_into_eq(rhs),
            (Self::Core(_), _) => None,
            (Self::Self_(_), _) => None,
        }
    }

    fn try_into_cmp(self, other: Self) -> Option<BoxedTemplateProperty<'a, Ordering>> {
        match (self, other) {
            (Self::Core(lhs), Self::Core(rhs)) => lhs.try_into_cmp(rhs),
            (Self::Core(_), _) => None,
            (Self::Self_(_), _) => None,
        }
    }
}

/// Function that translates keyword (or 0-ary method call node of the self type
/// `C`.)
///
/// Because the `GenericTemplateLanguage` doesn't provide a way to pass around
/// global resources, the keyword function is allowed to capture resources.
pub type GenericTemplateBuildKeywordFn<'a, C> = Box<
    dyn Fn(BoxedTemplateProperty<'a, C>) -> TemplateParseResult<GenericTemplatePropertyKind<'a, C>>
        + 'a,
>;

/// Table of functions that translate keyword node.
pub type GenericTemplateBuildKeywordFnMap<'a, C> =
    HashMap<&'static str, GenericTemplateBuildKeywordFn<'a, C>>;

/// Symbol table of methods available in the general-purpose template.
struct GenericTemplateBuildFnTable<'a, C> {
    core: CoreTemplateBuildFnTable<
        'a,
        GenericTemplateLanguage<'a, C>,
        GenericTemplatePropertyKind<'a, C>,
    >,
    keywords: GenericTemplateBuildKeywordFnMap<'a, C>,
}
