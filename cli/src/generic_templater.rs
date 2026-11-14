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

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use jj_lib::settings::UserSettings;
use jj_lib::template_parser;
use jj_lib::template_parser::FunctionCallNode;
use jj_lib::template_parser::TemplateDiagnostics;
use jj_lib::template_parser::TemplateParseResult;

use crate::template_builder;
use crate::template_builder::BuildContext;
use crate::template_builder::CoreTemplateBuildFnTable;
use crate::template_builder::CoreTemplatePropertyKind;
use crate::template_builder::CoreTemplatePropertyVar as _;
use crate::template_builder::TemplateLanguage;
use crate::template_builder::cast_value_property;
use crate::template_value::OpaqueTemplateValue;
use crate::template_value::TemplateTypeRegistry;
use crate::template_value::TemplateValue as _;
use crate::templater::BoxedTemplateProperty;
use crate::templater::TemplatePropertyExt as _;

/// General-purpose template language for basic value types.
///
/// This template language only supports the core template property types (plus
/// the self type `C`.) The self type `C` is usually a tuple or struct of value
/// types. It's cloned several times internally. Keyword functions need to be
/// registered to extract properties from the self object.
pub struct GenericTemplateLanguage<'a, C> {
    settings: UserSettings,
    current_dir: PathBuf,
    type_registry: TemplateTypeRegistry,
    build_fn_table: GenericTemplateBuildFnTable<'a, C>,
}

impl<'a, C> GenericTemplateLanguage<'a, C>
where
    C: OpaqueTemplateValue + serde::Serialize,
{
    /// Sets up environment with no keywords.
    ///
    /// New keyword functions can be registered by `add_keyword()`.
    pub fn new(settings: &UserSettings, current_dir: &Path) -> Self {
        Self::with_keywords(HashMap::new(), settings, current_dir)
    }

    /// Sets up environment with the given `keywords` table.
    pub fn with_keywords(
        keywords: GenericTemplateBuildKeywordFnMap<'a, C>,
        settings: &UserSettings,
        current_dir: &Path,
    ) -> Self {
        let mut type_registry = template_builder::new_core_type_registry();
        type_registry.register::<C>("Self").serializable();
        Self {
            // Clone settings to keep lifetime simple. It's cheap.
            settings: settings.clone(),
            current_dir: current_dir.to_owned(),
            type_registry,
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
        F: Fn(BoxedTemplateProperty<'a, C>) -> TemplateParseResult<CoreTemplatePropertyKind<'a>>
            + 'a,
    {
        self.build_fn_table.keywords.insert(name, Box::new(build));
    }
}

impl<'a, C> TemplateLanguage<'a> for GenericTemplateLanguage<'a, C>
where
    C: OpaqueTemplateValue + serde::Serialize,
{
    type Property = CoreTemplatePropertyKind<'a>;

    fn settings(&self) -> &UserSettings {
        &self.settings
    }

    fn current_dir(&self) -> &Path {
        &self.current_dir
    }

    fn type_registry(&self) -> &TemplateTypeRegistry {
        &self.type_registry
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
        let type_name = property.type_name(self.type_registry());
        match property {
            CoreTemplatePropertyKind::Value { ty, property } if ty == C::value_type() => {
                let table = &self.build_fn_table.keywords;
                let build = template_parser::lookup_method(type_name, table, function)?;
                // For simplicity, only 0-ary method is supported.
                function.expect_no_arguments()?;
                let self_property = cast_value_property::<C>(property, self.type_registry());
                build(self_property.into_dyn())
            }
            _ => {
                let table = &self.build_fn_table.core;
                table.build_method(self, diagnostics, build_ctx, property, function)
            }
        }
    }
}

/// Function that translates keyword (or 0-ary method call node of the self type
/// `C`.)
///
/// Because the `GenericTemplateLanguage` doesn't provide a way to pass around
/// global resources, the keyword function is allowed to capture resources.
pub type GenericTemplateBuildKeywordFn<'a, C> = Box<
    dyn Fn(BoxedTemplateProperty<'a, C>) -> TemplateParseResult<CoreTemplatePropertyKind<'a>> + 'a,
>;

/// Table of functions that translate keyword node.
pub type GenericTemplateBuildKeywordFnMap<'a, C> =
    HashMap<&'static str, GenericTemplateBuildKeywordFn<'a, C>>;

/// Symbol table of methods available in the general-purpose template.
struct GenericTemplateBuildFnTable<'a, C> {
    core:
        CoreTemplateBuildFnTable<'a, GenericTemplateLanguage<'a, C>, CoreTemplatePropertyKind<'a>>,
    keywords: GenericTemplateBuildKeywordFnMap<'a, C>,
}
