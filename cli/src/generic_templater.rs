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

use std::path::Path;
use std::path::PathBuf;

use jj_lib::settings::UserSettings;
use jj_lib::template_parser::FunctionCallNode;
use jj_lib::template_parser::TemplateDiagnostics;
use jj_lib::template_parser::TemplateParseResult;

use crate::template_builder;
use crate::template_builder::BuildContext;
use crate::template_builder::TemplateBuildFnTable;
use crate::template_builder::TemplateLanguage;
use crate::template_builder::TemplateMethodTableRegistration;
use crate::template_builder::TemplatePropertyKind;
use crate::template_value::OpaqueTemplateValue;
use crate::template_value::TemplateTypeRegistry;
use crate::template_value::TemplateValue;

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
    build_fn_table: TemplateBuildFnTable<'a, GenericTemplateLanguage<'a, C>>,
}

impl<'a, C> GenericTemplateLanguage<'a, C>
where
    C: OpaqueTemplateValue + serde::Serialize,
{
    /// Creates new template language environment.
    pub fn new(settings: &UserSettings, current_dir: &Path) -> Self {
        let mut type_registry = template_builder::new_core_type_registry();
        type_registry.register::<C>("Self").serializable();
        let mut build_fn_table = TemplateBuildFnTable::builtin();
        build_fn_table.register_methods::<C>();
        Self {
            // Clone settings to keep lifetime simple. It's cheap.
            settings: settings.clone(),
            current_dir: current_dir.to_owned(),
            type_registry,
            build_fn_table,
        }
    }

    /// Registers methods on `T`.
    #[track_caller]
    pub fn register_methods<T: TemplateValue>(
        &mut self,
    ) -> TemplateMethodTableRegistration<'a, '_, Self, T> {
        self.build_fn_table.register_methods::<T>()
    }
}

impl<'a, C> TemplateLanguage<'a> for GenericTemplateLanguage<'a, C>
where
    C: OpaqueTemplateValue + serde::Serialize,
{
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
        build_ctx: &BuildContext<'a, '_>,
        function: &FunctionCallNode,
    ) -> TemplateParseResult<TemplatePropertyKind<'a>> {
        let table = &self.build_fn_table;
        table.build_function(self, diagnostics, build_ctx, function)
    }

    fn build_method(
        &self,
        diagnostics: &mut TemplateDiagnostics,
        build_ctx: &BuildContext<'a, '_>,
        property: TemplatePropertyKind<'a>,
        function: &FunctionCallNode,
    ) -> TemplateParseResult<TemplatePropertyKind<'a>> {
        self.build_fn_table
            .build_method(self, diagnostics, build_ctx, property, function)
    }
}
