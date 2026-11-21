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

use crate::template_builder;
use crate::template_builder::TemplateBuildFnTable;
use crate::template_builder::TemplateLanguage;
use crate::template_builder::TemplateMethodTableRegistration;
use crate::template_value::TemplateTypeRegistration;
use crate::template_value::TemplateTypeRegistry;
use crate::template_value::TemplateValue;

/// General-purpose template language for basic value types.
pub struct GenericTemplateLanguage {
    settings: UserSettings,
    current_dir: PathBuf,
    type_registry: TemplateTypeRegistry,
    build_fn_table: TemplateBuildFnTable<'static, Self>,
}

impl GenericTemplateLanguage {
    /// Creates new template language environment.
    pub fn new(settings: &UserSettings, current_dir: &Path) -> Self {
        Self {
            // Clone settings to keep lifetime simple. It's cheap.
            settings: settings.clone(),
            current_dir: current_dir.to_owned(),
            type_registry: template_builder::new_core_type_registry(),
            build_fn_table: TemplateBuildFnTable::builtin(),
        }
    }

    /// Registers a new template value type.
    pub fn register_type<T: TemplateValue>(
        &mut self,
        type_name: &'static str,
    ) -> TemplateTypeRegistration<'_, T> {
        self.type_registry.register::<T>(type_name)
    }

    /// Registers methods on `T`.
    #[track_caller]
    pub fn register_methods<T: TemplateValue>(
        &mut self,
    ) -> TemplateMethodTableRegistration<'static, '_, Self, T> {
        self.build_fn_table.register_methods::<T>()
    }
}

impl TemplateLanguage<'static> for GenericTemplateLanguage {
    fn settings(&self) -> &UserSettings {
        &self.settings
    }

    fn current_dir(&self) -> &Path {
        &self.current_dir
    }

    fn type_registry(&self) -> &TemplateTypeRegistry {
        &self.type_registry
    }

    fn symbol_table(&self) -> &TemplateBuildFnTable<'static, Self> {
        &self.build_fn_table
    }
}
