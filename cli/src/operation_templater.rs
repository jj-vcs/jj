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

//! Template environment for `jj op log`.

use std::any::Any;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use itertools::Itertools as _;
use jj_lib::extensions_map::ExtensionsMap;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::OperationId;
use jj_lib::operation::Operation;
use jj_lib::repo::RepoLoader;
use jj_lib::settings::UserSettings;
use jj_lib::template_parser::FunctionCallNode;
use jj_lib::template_parser::TemplateDiagnostics;
use jj_lib::template_parser::TemplateParseError;
use jj_lib::template_parser::TemplateParseResult;
use pollster::FutureExt as _;

use crate::template_builder;
use crate::template_builder::BuildContext;
use crate::template_builder::BuildValueMethodFnMap;
use crate::template_builder::CoreTemplateBuildFnTable;
use crate::template_builder::CoreTemplatePropertyKind;
use crate::template_builder::TemplateLanguage;
use crate::template_builder::builtin_unformattable_value_list_methods;
use crate::template_builder::cast_value_property;
use crate::template_value::OpaqueTemplateValue;
use crate::template_value::TemplateTypeRegistry;
use crate::template_value::TemplateValue as _;
use crate::templater::NoTemplateBooleanCast;
use crate::templater::Template;
use crate::templater::TemplateFormatter;
use crate::templater::TemplatePropertyExt as _;
use crate::templater::WrapTemplateProperty;

pub trait OperationTemplateLanguageExtension {
    fn build_fn_table(&self) -> CoreTemplateBuildFnTable<'static, OperationTemplateLanguage>;

    fn build_cache_extensions(&self, extensions: &mut ExtensionsMap);
}

/// Global resources needed by operation template methods.
pub trait OperationTemplateEnvironment {
    fn repo_loader(&self) -> &RepoLoader;
    fn current_op_id(&self) -> Option<&OperationId>;
}

/// Template environment for `jj op log`.
pub struct OperationTemplateLanguage {
    repo_loader: RepoLoader,
    current_op_id: Option<OperationId>,
    current_dir: PathBuf,
    type_registry: TemplateTypeRegistry,
    build_fn_table: CoreTemplateBuildFnTable<'static, Self>,
    cache_extensions: ExtensionsMap,
}

impl OperationTemplateLanguage {
    /// Sets up environment where operation template will be transformed to
    /// evaluation tree.
    pub fn new(
        repo_loader: &RepoLoader,
        current_op_id: Option<&OperationId>,
        current_dir: &Path,
        extensions: &[impl AsRef<dyn OperationTemplateLanguageExtension>],
    ) -> Self {
        let mut type_registry = template_builder::new_core_type_registry();
        register_types(&mut type_registry);
        let mut build_fn_table = CoreTemplateBuildFnTable::builtin();
        register_methods(&mut build_fn_table);

        let mut cache_extensions = ExtensionsMap::empty();
        for extension in extensions {
            build_fn_table.merge(extension.as_ref().build_fn_table());
            extension
                .as_ref()
                .build_cache_extensions(&mut cache_extensions);
        }

        Self {
            // Clone these to keep lifetime simple
            repo_loader: repo_loader.clone(),
            current_op_id: current_op_id.cloned(),
            current_dir: current_dir.to_owned(),
            type_registry,
            build_fn_table,
            cache_extensions,
        }
    }
}

impl TemplateLanguage<'static> for OperationTemplateLanguage {
    type Property = CoreTemplatePropertyKind<'static>;

    fn settings(&self) -> &UserSettings {
        self.repo_loader.settings()
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
        let table = &self.build_fn_table;
        table.build_function(self, diagnostics, build_ctx, function)
    }

    fn build_method(
        &self,
        diagnostics: &mut TemplateDiagnostics,
        build_ctx: &BuildContext<Self::Property>,
        property: Self::Property,
        function: &FunctionCallNode,
    ) -> TemplateParseResult<Self::Property> {
        self.build_fn_table
            .build_method(self, diagnostics, build_ctx, property, function)
    }
}

impl OperationTemplateEnvironment for OperationTemplateLanguage {
    fn repo_loader(&self) -> &RepoLoader {
        &self.repo_loader
    }

    fn current_op_id(&self) -> Option<&OperationId> {
        self.current_op_id.as_ref()
    }
}

impl OperationTemplateLanguage {
    pub fn cache_extension<T: Any>(&self) -> Option<&T> {
        self.cache_extensions.get::<T>()
    }
}

/// Wrapper for the operation template property types.
pub trait OperationTemplatePropertyVar<'a>
where
    Self: WrapTemplateProperty<'a, Operation>,
    Self: WrapTemplateProperty<'a, Option<Operation>>,
    Self: WrapTemplateProperty<'a, Vec<Operation>>,
    Self: WrapTemplateProperty<'a, OperationId>,
{
}

impl OpaqueTemplateValue for Operation {}
impl OpaqueTemplateValue for OperationId {}

impl NoTemplateBooleanCast for Operation {}
impl NoTemplateBooleanCast for OperationId {}

impl<'a> OperationTemplatePropertyVar<'a> for CoreTemplatePropertyKind<'a> {}

pub fn register_types(registry: &mut TemplateTypeRegistry) {
    registry.register::<Operation>("Operation").serializable();
    registry
        .register::<Option<Operation>>("Option<Operation>")
        .conditionable()
        .serializable();
    registry
        .register::<Vec<Operation>>("List<Operation>")
        .conditionable()
        .serializable();
    registry
        .register::<OperationId>("OperationId")
        .formattable()
        .serializable();
}

pub fn register_methods<'a, L>(table: &mut CoreTemplateBuildFnTable<'a, L>)
where
    L: TemplateLanguage<'a> + OperationTemplateEnvironment + ?Sized,
    L::Property: OperationTemplatePropertyVar<'a>,
{
    let methods = &mut table.methods;
    methods.insert(Operation::value_type(), builtin_operation_methods());
    methods.insert(
        <Vec<Operation>>::value_type(),
        builtin_unformattable_value_list_methods::<_, Operation>(),
    );
    methods.insert(OperationId::value_type(), builtin_operation_id_methods());
}

fn builtin_operation_methods<'a, L>() -> BuildValueMethodFnMap<'a, L>
where
    L: TemplateLanguage<'a> + OperationTemplateEnvironment + ?Sized,
    L::Property: OperationTemplatePropertyVar<'a>,
{
    type T = Operation;
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = BuildValueMethodFnMap::<L>::new();
    map.insert(
        "current_operation",
        |language, _diagnostics, _build_ctx, self_property, function| {
            let self_property = cast_value_property::<T>(self_property, language.type_registry());
            function.expect_no_arguments()?;
            let current_op_id = language.current_op_id().cloned();
            let out_property = self_property.map(move |op| Some(op.id()) == current_op_id.as_ref());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "description",
        |language, _diagnostics, _build_ctx, self_property, function| {
            let self_property = cast_value_property::<T>(self_property, language.type_registry());
            function.expect_no_arguments()?;
            let out_property = self_property.map(|op| op.metadata().description.clone());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "id",
        |language, _diagnostics, _build_ctx, self_property, function| {
            let self_property = cast_value_property::<T>(self_property, language.type_registry());
            function.expect_no_arguments()?;
            let out_property = self_property.map(|op| op.id().clone());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "attributes",
        |language, _diagnostics, _build_ctx, self_property, function| {
            let self_property = cast_value_property::<T>(self_property, language.type_registry());
            function.expect_no_arguments()?;
            let out_property = self_property.map(|op| {
                // TODO: introduce map type
                op.metadata()
                    .attributes
                    .iter()
                    .map(|(key, value)| format!("{key}: {value}"))
                    .join("\n")
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    // TODO: Remove in jj 0.47+
    map.insert(
        "tags",
        |language, diagnostics, _build_ctx, self_property, function| {
            let self_property = cast_value_property::<T>(self_property, language.type_registry());
            diagnostics.add_warning(TemplateParseError::expression(
                "operation.tags() is deprecated; use .attributes() instead",
                function.name_span,
            ));
            function.expect_no_arguments()?;
            let out_property = self_property.map(|op| {
                // TODO: introduce map type
                op.metadata()
                    .attributes
                    .iter()
                    .map(|(key, value)| format!("{key}: {value}"))
                    .join("\n")
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "snapshot",
        |language, _diagnostics, _build_ctx, self_property, function| {
            let self_property = cast_value_property::<T>(self_property, language.type_registry());
            function.expect_no_arguments()?;
            let out_property = self_property.map(|op| op.metadata().is_snapshot);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "workspace_name",
        |language, _diagnostics, _build_ctx, self_property, function| {
            let self_property = cast_value_property::<T>(self_property, language.type_registry());
            function.expect_no_arguments()?;
            let out_property = self_property.map(|op| {
                op.metadata()
                    .workspace_name
                    .as_ref()
                    .map(|name| format!("{}@", name.as_symbol()))
                    .unwrap_or_default()
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "time",
        |language, _diagnostics, _build_ctx, self_property, function| {
            let self_property = cast_value_property::<T>(self_property, language.type_registry());
            function.expect_no_arguments()?;
            let out_property = self_property.map(|op| op.metadata().time.clone());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "user",
        |language, _diagnostics, _build_ctx, self_property, function| {
            let self_property = cast_value_property::<T>(self_property, language.type_registry());
            function.expect_no_arguments()?;
            let out_property = self_property.map(|op| {
                // TODO: introduce dedicated type and provide accessors?
                format!("{}@{}", op.metadata().username, op.metadata().hostname)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "root",
        |language, _diagnostics, _build_ctx, self_property, function| {
            let self_property = cast_value_property::<T>(self_property, language.type_registry());
            function.expect_no_arguments()?;
            let op_store = language.repo_loader().op_store();
            let root_op_id = op_store.root_operation_id().clone();
            let out_property = self_property.map(move |op| op.id() == &root_op_id);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "parents",
        |language, _diagnostics, _build_ctx, self_property, function| {
            let self_property = cast_value_property::<T>(self_property, language.type_registry());
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|op| {
                let ops = op.parents().block_on()?;
                Ok(ops)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

impl Template for OperationId {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        write!(formatter, "{}", self.hex())
    }
}

fn builtin_operation_id_methods<'a, L>() -> BuildValueMethodFnMap<'a, L>
where
    L: TemplateLanguage<'a> + OperationTemplateEnvironment + ?Sized,
    L::Property: OperationTemplatePropertyVar<'a>,
{
    type T = OperationId;
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = BuildValueMethodFnMap::<L>::new();
    map.insert(
        "short",
        |language, diagnostics, build_ctx, self_property, function| {
            let self_property = cast_value_property::<T>(self_property, language.type_registry());
            let ([], [len_node]) = function.expect_arguments()?;
            let len_property = len_node
                .map(|node| {
                    template_builder::expect_usize_expression(
                        language,
                        diagnostics,
                        build_ctx,
                        node,
                    )
                })
                .transpose()?;
            let out_property = (self_property, len_property).map(|(id, len)| {
                let mut hex = id.hex();
                hex.truncate(len.unwrap_or(12));
                hex
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}
