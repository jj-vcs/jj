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
use std::cmp::Ordering;
use std::collections::HashMap;
use std::io;

use itertools::Itertools as _;
use jj_lib::extensions_map::ExtensionsMap;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::OperationId;
use jj_lib::operation::Operation;
use jj_lib::repo::RepoLoader;
use jj_lib::settings::UserSettings;

use crate::template_builder;
use crate::template_builder::BuildContext;
use crate::template_builder::CoreTemplateBuildFnTable;
use crate::template_builder::CoreTemplatePropertyKind;
use crate::template_builder::CoreTemplatePropertyVar;
use crate::template_builder::TemplateBuildMethodFnMap;
use crate::template_builder::TemplateLanguage;
use crate::template_builder::merge_fn_map;
use crate::template_parser;
use crate::template_parser::FunctionCallNode;
use crate::template_parser::TemplateDiagnostics;
use crate::template_parser::TemplateParseResult;
use crate::templater::BoxedSerializeProperty;
use crate::templater::BoxedTemplateProperty;
use crate::templater::ListTemplate;
use crate::templater::PlainTextFormattedProperty;
use crate::templater::Template;
use crate::templater::TemplateFormatter;
use crate::templater::TemplatePropertyExt as _;
use crate::templater::WrapTemplateProperty;

pub trait OperationTemplateLanguageExtension {
    fn build_fn_table(&self) -> OperationTemplateLanguageBuildFnTable;

    fn build_cache_extensions(&self, extensions: &mut ExtensionsMap);
}

/// Global resources needed by [`OperationTemplatePropertyKind`] methods.
pub trait OperationTemplateEnvironment {
    fn repo_loader(&self) -> &RepoLoader;
    fn current_op_id(&self) -> Option<&OperationId>;
}

/// Template environment for `jj op log`.
pub struct OperationTemplateLanguage {
    repo_loader: RepoLoader,
    current_op_id: Option<OperationId>,
    build_fn_table: OperationTemplateLanguageBuildFnTable,
    cache_extensions: ExtensionsMap,
}

impl OperationTemplateLanguage {
    /// Sets up environment where operation template will be transformed to
    /// evaluation tree.
    pub fn new(
        repo_loader: &RepoLoader,
        current_op_id: Option<&OperationId>,
        extensions: &[impl AsRef<dyn OperationTemplateLanguageExtension>],
    ) -> Self {
        let mut build_fn_table = OperationTemplateLanguageBuildFnTable::builtin();
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
            build_fn_table,
            cache_extensions,
        }
    }
}

impl TemplateLanguage<'static> for OperationTemplateLanguage {
    type Property = OperationTemplateLanguagePropertyKind;

    fn settings(&self) -> &UserSettings {
        self.repo_loader.settings()
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
        match property {
            OperationTemplateLanguagePropertyKind::Core(property) => {
                let table = &self.build_fn_table.core;
                table.build_method(self, diagnostics, build_ctx, property, function)
            }
            OperationTemplateLanguagePropertyKind::Operation(property) => {
                let table = &self.build_fn_table.operation;
                table.build_method(self, diagnostics, build_ctx, property, function)
            }
        }
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

/// Tagged union of the operation template property types.
pub enum OperationTemplatePropertyKind<'a> {
    Operation(BoxedTemplateProperty<'a, Operation>),
    OperationOpt(BoxedTemplateProperty<'a, Option<Operation>>),
    OperationList(BoxedTemplateProperty<'a, Vec<Operation>>),
    OperationId(BoxedTemplateProperty<'a, OperationId>),
}

/// Implements `WrapTemplateProperty<type>` for operation property types.
///
/// Use `impl_operation_property_wrappers!(<'a> Kind<'a> => Operation);` to
/// implement forwarding conversion.
macro_rules! impl_operation_property_wrappers {
    ($($head:tt)+) => {
        $crate::template_builder::impl_property_wrappers!($($head)+ {
            Operation(jj_lib::operation::Operation),
            OperationOpt(Option<jj_lib::operation::Operation>),
            OperationList(Vec<jj_lib::operation::Operation>),
            OperationId(jj_lib::op_store::OperationId),
        });
    };
}

pub(crate) use impl_operation_property_wrappers;

impl_operation_property_wrappers!(<'a> OperationTemplatePropertyKind<'a>);

impl<'a> OperationTemplatePropertyKind<'a> {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Operation(_) => "Operation",
            Self::OperationOpt(_) => "Option<Operation>",
            Self::OperationList(_) => "List<Operation>",
            Self::OperationId(_) => "OperationId",
        }
    }

    pub fn try_into_boolean(self) -> Option<BoxedTemplateProperty<'a, bool>> {
        match self {
            Self::Operation(_) => None,
            Self::OperationOpt(property) => Some(property.map(|opt| opt.is_some()).into_dyn()),
            Self::OperationList(property) => Some(property.map(|l| !l.is_empty()).into_dyn()),
            Self::OperationId(_) => None,
        }
    }

    pub fn try_into_integer(self) -> Option<BoxedTemplateProperty<'a, i64>> {
        None
    }

    pub fn try_into_stringify(self) -> Option<BoxedTemplateProperty<'a, String>> {
        let template = self.try_into_template()?;
        Some(PlainTextFormattedProperty::new(template).into_dyn())
    }

    pub fn try_into_serialize(self) -> Option<BoxedSerializeProperty<'a>> {
        match self {
            Self::Operation(property) => Some(property.into_serialize()),
            Self::OperationOpt(property) => Some(property.into_serialize()),
            Self::OperationList(property) => Some(property.into_serialize()),
            Self::OperationId(property) => Some(property.into_serialize()),
        }
    }

    pub fn try_into_template(self) -> Option<Box<dyn Template + 'a>> {
        match self {
            Self::Operation(_) => None,
            Self::OperationOpt(_) => None,
            Self::OperationList(_) => None,
            Self::OperationId(property) => Some(property.into_template()),
        }
    }

    pub fn try_into_eq(self, other: Self) -> Option<BoxedTemplateProperty<'a, bool>> {
        match (self, other) {
            (Self::Operation(_), _) => None,
            (Self::OperationOpt(_), _) => None,
            (Self::OperationList(_), _) => None,
            (Self::OperationId(_), _) => None,
        }
    }

    pub fn try_into_eq_core(
        self,
        other: CoreTemplatePropertyKind<'a>,
    ) -> Option<BoxedTemplateProperty<'a, bool>> {
        match (self, other) {
            (Self::Operation(_), _) => None,
            (Self::OperationOpt(_), _) => None,
            (Self::OperationList(_), _) => None,
            (Self::OperationId(_), _) => None,
        }
    }

    pub fn try_into_cmp(self, other: Self) -> Option<BoxedTemplateProperty<'a, Ordering>> {
        match (self, other) {
            (Self::Operation(_), _) => None,
            (Self::OperationOpt(_), _) => None,
            (Self::OperationList(_), _) => None,
            (Self::OperationId(_), _) => None,
        }
    }

    pub fn try_into_cmp_core(
        self,
        other: CoreTemplatePropertyKind<'a>,
    ) -> Option<BoxedTemplateProperty<'a, Ordering>> {
        match (self, other) {
            (Self::Operation(_), _) => None,
            (Self::OperationOpt(_), _) => None,
            (Self::OperationList(_), _) => None,
            (Self::OperationId(_), _) => None,
        }
    }
}

/// Tagged property types available in [`OperationTemplateLanguage`].
pub enum OperationTemplateLanguagePropertyKind {
    Core(CoreTemplatePropertyKind<'static>),
    Operation(OperationTemplatePropertyKind<'static>),
}

template_builder::impl_core_property_wrappers!(OperationTemplateLanguagePropertyKind => Core);
impl_operation_property_wrappers!(OperationTemplateLanguagePropertyKind => Operation);

impl CoreTemplatePropertyVar<'static> for OperationTemplateLanguagePropertyKind {
    fn wrap_template(template: Box<dyn Template>) -> Self {
        Self::Core(CoreTemplatePropertyKind::wrap_template(template))
    }

    fn wrap_list_template(template: Box<dyn ListTemplate>) -> Self {
        Self::Core(CoreTemplatePropertyKind::wrap_list_template(template))
    }

    fn wrap_json_value(property: BoxedSerializeProperty<'static>) -> Self {
        Self::Core(CoreTemplatePropertyKind::wrap_json_value(property))
    }

    fn type_name(&self) -> &'static str {
        match self {
            Self::Core(property) => property.type_name(),
            Self::Operation(property) => property.type_name(),
        }
    }

    fn try_into_boolean(self) -> Option<BoxedTemplateProperty<'static, bool>> {
        match self {
            Self::Core(property) => property.try_into_boolean(),
            Self::Operation(property) => property.try_into_boolean(),
        }
    }

    fn try_into_integer(self) -> Option<BoxedTemplateProperty<'static, i64>> {
        match self {
            Self::Core(property) => property.try_into_integer(),
            Self::Operation(property) => property.try_into_integer(),
        }
    }

    fn try_into_stringify(self) -> Option<BoxedTemplateProperty<'static, String>> {
        match self {
            Self::Core(property) => property.try_into_stringify(),
            Self::Operation(property) => property.try_into_stringify(),
        }
    }

    fn try_into_serialize(self) -> Option<BoxedSerializeProperty<'static>> {
        match self {
            Self::Core(property) => property.try_into_serialize(),
            Self::Operation(property) => property.try_into_serialize(),
        }
    }

    fn try_into_template(self) -> Option<Box<dyn Template>> {
        match self {
            Self::Core(property) => property.try_into_template(),
            Self::Operation(property) => property.try_into_template(),
        }
    }

    fn try_into_eq(self, other: Self) -> Option<BoxedTemplateProperty<'static, bool>> {
        match (self, other) {
            (Self::Core(lhs), Self::Core(rhs)) => lhs.try_into_eq(rhs),
            (Self::Core(lhs), Self::Operation(rhs)) => rhs.try_into_eq_core(lhs),
            (Self::Operation(lhs), Self::Core(rhs)) => lhs.try_into_eq_core(rhs),
            (Self::Operation(lhs), Self::Operation(rhs)) => lhs.try_into_eq(rhs),
        }
    }

    fn try_into_cmp(self, other: Self) -> Option<BoxedTemplateProperty<'static, Ordering>> {
        match (self, other) {
            (Self::Core(lhs), Self::Core(rhs)) => lhs.try_into_cmp(rhs),
            (Self::Core(lhs), Self::Operation(rhs)) => rhs
                .try_into_cmp_core(lhs)
                .map(|property| property.map(Ordering::reverse).into_dyn()),
            (Self::Operation(lhs), Self::Core(rhs)) => lhs.try_into_cmp_core(rhs),
            (Self::Operation(lhs), Self::Operation(rhs)) => lhs.try_into_cmp(rhs),
        }
    }
}

impl OperationTemplatePropertyVar<'static> for OperationTemplateLanguagePropertyKind {}

/// Symbol table for the operation template property types.
pub struct OperationTemplateBuildFnTable<'a, L: ?Sized, P = <L as TemplateLanguage<'a>>::Property> {
    pub operation_methods: TemplateBuildMethodFnMap<'a, L, Operation, P>,
    pub operation_list_methods: TemplateBuildMethodFnMap<'a, L, Vec<Operation>, P>,
    pub operation_id_methods: TemplateBuildMethodFnMap<'a, L, OperationId, P>,
}

impl<L: ?Sized, P> OperationTemplateBuildFnTable<'_, L, P> {
    pub fn empty() -> Self {
        Self {
            operation_methods: HashMap::new(),
            operation_list_methods: HashMap::new(),
            operation_id_methods: HashMap::new(),
        }
    }

    pub fn merge(&mut self, other: Self) {
        let Self {
            operation_methods,
            operation_list_methods,
            operation_id_methods,
        } = other;

        merge_fn_map(&mut self.operation_methods, operation_methods);
        merge_fn_map(&mut self.operation_list_methods, operation_list_methods);
        merge_fn_map(&mut self.operation_id_methods, operation_id_methods);
    }
}

impl<'a, L> OperationTemplateBuildFnTable<'a, L, L::Property>
where
    L: TemplateLanguage<'a> + OperationTemplateEnvironment + ?Sized,
    L::Property: OperationTemplatePropertyVar<'a>,
{
    /// Creates new symbol table containing the builtin methods.
    pub fn builtin() -> Self {
        Self {
            operation_methods: builtin_operation_methods(),
            operation_list_methods: template_builder::builtin_unformattable_list_methods(),
            operation_id_methods: builtin_operation_id_methods(),
        }
    }

    /// Applies the method call node `function` to the given `property` by using
    /// this symbol table.
    pub fn build_method(
        &self,
        language: &L,
        diagnostics: &mut TemplateDiagnostics,
        build_ctx: &BuildContext<L::Property>,
        property: OperationTemplatePropertyKind<'a>,
        function: &FunctionCallNode,
    ) -> TemplateParseResult<L::Property> {
        let type_name = property.type_name();
        match property {
            OperationTemplatePropertyKind::Operation(property) => {
                let table = &self.operation_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(language, diagnostics, build_ctx, property, function)
            }
            OperationTemplatePropertyKind::OperationOpt(property) => {
                let type_name = "Operation";
                let table = &self.operation_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                let inner_property = property.try_unwrap(type_name).into_dyn();
                build(language, diagnostics, build_ctx, inner_property, function)
            }
            OperationTemplatePropertyKind::OperationList(property) => {
                let table = &self.operation_list_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(language, diagnostics, build_ctx, property, function)
            }
            OperationTemplatePropertyKind::OperationId(property) => {
                let table = &self.operation_id_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(language, diagnostics, build_ctx, property, function)
            }
        }
    }
}

/// Symbol table of methods available in [`OperationTemplateLanguage`].
pub struct OperationTemplateLanguageBuildFnTable {
    pub core: CoreTemplateBuildFnTable<'static, OperationTemplateLanguage>,
    pub operation: OperationTemplateBuildFnTable<'static, OperationTemplateLanguage>,
}

impl OperationTemplateLanguageBuildFnTable {
    pub fn empty() -> Self {
        Self {
            core: CoreTemplateBuildFnTable::empty(),
            operation: OperationTemplateBuildFnTable::empty(),
        }
    }

    fn merge(&mut self, other: Self) {
        let Self { core, operation } = other;

        self.core.merge(core);
        self.operation.merge(operation);
    }

    /// Creates new symbol table containing the builtin methods.
    fn builtin() -> Self {
        Self {
            core: CoreTemplateBuildFnTable::builtin(),
            operation: OperationTemplateBuildFnTable::builtin(),
        }
    }
}

fn builtin_operation_methods<'a, L>() -> TemplateBuildMethodFnMap<'a, L, Operation>
where
    L: TemplateLanguage<'a> + OperationTemplateEnvironment + ?Sized,
    L::Property: OperationTemplatePropertyVar<'a>,
{
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = TemplateBuildMethodFnMap::<L, Operation>::new();
    map.insert(
        "current_operation",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let current_op_id = language.current_op_id().cloned();
            let out_property = self_property.map(move |op| Some(op.id()) == current_op_id.as_ref());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "description",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|op| op.metadata().description.clone());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "id",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|op| op.id().clone());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "tags",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|op| {
                // TODO: introduce map type
                op.metadata()
                    .tags
                    .iter()
                    .map(|(key, value)| format!("{key}: {value}"))
                    .join("\n")
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "snapshot",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|op| op.metadata().is_snapshot);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "time",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|op| op.metadata().time.clone());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "user",
        |_language, _diagnostics, _build_ctx, self_property, function| {
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
            function.expect_no_arguments()?;
            let op_store = language.repo_loader().op_store();
            let root_op_id = op_store.root_operation_id().clone();
            let out_property = self_property.map(move |op| op.id() == &root_op_id);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "parents",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|op| {
                let ops: Vec<_> = op.parents().try_collect()?;
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

fn builtin_operation_id_methods<'a, L>() -> TemplateBuildMethodFnMap<'a, L, OperationId>
where
    L: TemplateLanguage<'a> + OperationTemplateEnvironment + ?Sized,
    L::Property: OperationTemplatePropertyVar<'a>,
{
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = TemplateBuildMethodFnMap::<L, OperationId>::new();
    map.insert(
        "short",
        |language, diagnostics, build_ctx, self_property, function| {
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
