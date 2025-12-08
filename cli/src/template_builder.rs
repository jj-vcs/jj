// Copyright 2020-2023 The Jujutsu Authors
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
use std::io;
use std::iter;

use itertools::Itertools as _;
use jj_lib::backend::Signature;
use jj_lib::backend::Timestamp;
use jj_lib::config::ConfigNamePathBuf;
use jj_lib::config::ConfigValue;
use jj_lib::content_hash::blake2b_hash;
use jj_lib::hex_util;
use jj_lib::op_store::TimestampRange;
use jj_lib::settings::UserSettings;
use jj_lib::time_util::DatePattern;
use serde::Deserialize;
use serde::de::IntoDeserializer as _;

use crate::config;
use crate::formatter::FormatRecorder;
use crate::formatter::Formatter;
use crate::template_parser;
use crate::template_parser::BinaryOp;
use crate::template_parser::ExpressionKind;
use crate::template_parser::ExpressionNode;
use crate::template_parser::FunctionCallNode;
use crate::template_parser::LambdaNode;
use crate::template_parser::TemplateAliasesMap;
use crate::template_parser::TemplateDiagnostics;
use crate::template_parser::TemplateParseError;
use crate::template_parser::TemplateParseErrorKind;
use crate::template_parser::TemplateParseResult;
use crate::template_parser::UnaryOp;
use crate::templater::BoxedSerializeProperty;
use crate::templater::BoxedTemplateProperty;
use crate::templater::CoalesceTemplate;
use crate::templater::ConcatTemplate;
use crate::templater::ConditionalTemplate;
use crate::templater::Email;
use crate::templater::JoinTemplate;
use crate::templater::LabelTemplate;
use crate::templater::ListPropertyTemplate;
use crate::templater::ListTemplate;
use crate::templater::Literal;
use crate::templater::PlainTextFormattedProperty;
use crate::templater::PropertyPlaceholder;
use crate::templater::RawEscapeSequenceTemplate;
use crate::templater::ReformatTemplate;
use crate::templater::SeparateTemplate;
use crate::templater::SizeHint;
use crate::templater::Template;
use crate::templater::TemplateProperty;
use crate::templater::TemplatePropertyError;
use crate::templater::TemplatePropertyExt as _;
use crate::templater::TemplateRenderer;
use crate::templater::WrapTemplateProperty;
use crate::text_util;
use crate::time_util;

/// Callbacks to build usage-context-specific evaluation objects from AST nodes.
///
/// This is used to implement different meanings of `self` or different
/// globally available functions in the template language depending on the
/// context in which it is invoked.
pub trait TemplateLanguage<'a> {
    type Property: CoreTemplatePropertyVar<'a>;

    fn settings(&self) -> &UserSettings;

    /// Translates the given global `function` call to a property.
    ///
    /// This should be delegated to
    /// `CoreTemplateBuildFnTable::build_function()`.
    fn build_function(
        &self,
        diagnostics: &mut TemplateDiagnostics,
        build_ctx: &BuildContext<Self::Property>,
        function: &FunctionCallNode,
    ) -> TemplateParseResult<Self::Property>;

    /// Creates a method call thunk for the given `function` of the given
    /// `property`.
    fn build_method(
        &self,
        diagnostics: &mut TemplateDiagnostics,
        build_ctx: &BuildContext<Self::Property>,
        property: Self::Property,
        function: &FunctionCallNode,
    ) -> TemplateParseResult<Self::Property>;
}

/// Implements [`WrapTemplateProperty<'a, O>`] for property types.
///
/// - `impl_property_wrappers!(Kind { Foo(Foo), FooList(Vec<Foo>), .. });` to
///   implement conversion from types `Foo`, `Vec<Foo>`, ...
/// - `impl_property_wrappers!(<'a> Kind<'a> { .. });` for types with lifetime.
/// - `impl_property_wrappers!(Kind => Core { .. });` to forward conversion to
///   `Kind::Core(_)`.
macro_rules! impl_property_wrappers {
    ($kind:path $(=> $var:ident)? { $($body:tt)* }) => {
        $crate::template_builder::_impl_property_wrappers_many!(
            [], 'static, $kind $(=> $var)?, { $($body)* });
    };
    // capture the first lifetime as the lifetime of template objects.
    (<$a:lifetime $(, $p:lifetime)* $(, $q:ident)*>
     $kind:path $(=> $var:ident)? { $($body:tt)* }) => {
        $crate::template_builder::_impl_property_wrappers_many!(
            [$a, $($p,)* $($q,)*], $a, $kind $(=> $var)?, { $($body)* });
    };
}

macro_rules! _impl_property_wrappers_many {
    // lifetime/type parameters are packed in order to disable zipping.
    // https://github.com/rust-lang/rust/issues/96184#issuecomment-1294999418
    ($ps:tt, $a:lifetime, $kind:path, { $( $var:ident($ty:ty), )* }) => {
        $(
            $crate::template_builder::_impl_property_wrappers_one!(
                $ps, $a, $kind, $var, $ty, std::convert::identity);
        )*
    };
    // variant part in body is ignored so the same body can be reused for
    // implementing forwarding conversion.
    ($ps:tt, $a:lifetime, $kind:path => $var:ident, { $( $ignored_var:ident($ty:ty), )* }) => {
        $(
            $crate::template_builder::_impl_property_wrappers_one!(
                $ps, $a, $kind, $var, $ty, $crate::templater::WrapTemplateProperty::wrap_property);
        )*
    };
}

macro_rules! _impl_property_wrappers_one {
    ([$($p:tt)*], $a:lifetime, $kind:path, $var:ident, $ty:ty, $inner:path) => {
        impl<$($p)*> $crate::templater::WrapTemplateProperty<$a, $ty> for $kind {
            fn wrap_property(property: $crate::templater::BoxedTemplateProperty<$a, $ty>) -> Self {
                Self::$var($inner(property))
            }
        }
    };
}

pub(crate) use _impl_property_wrappers_many;
pub(crate) use _impl_property_wrappers_one;
pub(crate) use impl_property_wrappers;

/// Wrapper for the core template property types.
pub trait CoreTemplatePropertyVar<'a>
where
    Self: WrapTemplateProperty<'a, String>,
    Self: WrapTemplateProperty<'a, Vec<String>>,
    Self: WrapTemplateProperty<'a, bool>,
    Self: WrapTemplateProperty<'a, i64>,
    Self: WrapTemplateProperty<'a, Option<i64>>,
    Self: WrapTemplateProperty<'a, ConfigValue>,
    Self: WrapTemplateProperty<'a, Signature>,
    Self: WrapTemplateProperty<'a, Email>,
    Self: WrapTemplateProperty<'a, SizeHint>,
    Self: WrapTemplateProperty<'a, Timestamp>,
    Self: WrapTemplateProperty<'a, TimestampRange>,
{
    fn wrap_template(template: Box<dyn Template + 'a>) -> Self;
    fn wrap_list_template(template: Box<dyn ListTemplate + 'a>) -> Self;

    /// Type name of the property output.
    fn type_name(&self) -> &'static str;

    fn try_into_boolean(self) -> Option<BoxedTemplateProperty<'a, bool>>;
    fn try_into_integer(self) -> Option<BoxedTemplateProperty<'a, i64>>;

    /// Transforms into a string property by formatting the value if needed.
    fn try_into_stringify(self) -> Option<BoxedTemplateProperty<'a, String>>;
    fn try_into_serialize(self) -> Option<BoxedSerializeProperty<'a>>;
    fn try_into_template(self) -> Option<Box<dyn Template + 'a>>;

    /// Transforms into a property that will evaluate to `self == other`.
    fn try_into_eq(self, other: Self) -> Option<BoxedTemplateProperty<'a, bool>>;

    /// Transforms into a property that will evaluate to an [`Ordering`].
    fn try_into_cmp(self, other: Self) -> Option<BoxedTemplateProperty<'a, Ordering>>;
}

pub enum CoreTemplatePropertyKind<'a> {
    String(BoxedTemplateProperty<'a, String>),
    StringList(BoxedTemplateProperty<'a, Vec<String>>),
    Boolean(BoxedTemplateProperty<'a, bool>),
    Integer(BoxedTemplateProperty<'a, i64>),
    IntegerOpt(BoxedTemplateProperty<'a, Option<i64>>),
    ConfigValue(BoxedTemplateProperty<'a, ConfigValue>),
    Signature(BoxedTemplateProperty<'a, Signature>),
    Email(BoxedTemplateProperty<'a, Email>),
    SizeHint(BoxedTemplateProperty<'a, SizeHint>),
    Timestamp(BoxedTemplateProperty<'a, Timestamp>),
    TimestampRange(BoxedTemplateProperty<'a, TimestampRange>),

    // Both TemplateProperty and Template can represent a value to be evaluated
    // dynamically, which suggests that `Box<dyn Template + 'a>` could be
    // composed as `Box<dyn TemplateProperty<Output = Box<dyn Template ..`.
    // However, there's a subtle difference: TemplateProperty is strict on
    // error, whereas Template is usually lax and prints an error inline. If
    // `concat(x, y)` were a property returning Template, and if `y` failed to
    // evaluate, the whole expression would fail. In this example, a partial
    // evaluation output is more useful. That's one reason why Template isn't
    // wrapped in a TemplateProperty. Another reason is that the outermost
    // caller expects a Template, not a TemplateProperty of Template output.
    Template(Box<dyn Template + 'a>),
    ListTemplate(Box<dyn ListTemplate + 'a>),
}

/// Implements `WrapTemplateProperty<type>` for core property types.
///
/// Use `impl_core_property_wrappers!(<'a> Kind<'a> => Core);` to implement
/// forwarding conversion.
macro_rules! impl_core_property_wrappers {
    ($($head:tt)+) => {
        $crate::template_builder::impl_property_wrappers!($($head)+ {
            String(String),
            StringList(Vec<String>),
            Boolean(bool),
            Integer(i64),
            IntegerOpt(Option<i64>),
            ConfigValue(jj_lib::config::ConfigValue),
            Signature(jj_lib::backend::Signature),
            Email($crate::templater::Email),
            SizeHint($crate::templater::SizeHint),
            Timestamp(jj_lib::backend::Timestamp),
            TimestampRange(jj_lib::op_store::TimestampRange),
        });
    };
}

pub(crate) use impl_core_property_wrappers;

impl_core_property_wrappers!(<'a> CoreTemplatePropertyKind<'a>);

impl<'a> CoreTemplatePropertyVar<'a> for CoreTemplatePropertyKind<'a> {
    fn wrap_template(template: Box<dyn Template + 'a>) -> Self {
        Self::Template(template)
    }

    fn wrap_list_template(template: Box<dyn ListTemplate + 'a>) -> Self {
        Self::ListTemplate(template)
    }

    fn type_name(&self) -> &'static str {
        match self {
            Self::String(_) => "String",
            Self::StringList(_) => "List<String>",
            Self::Boolean(_) => "Boolean",
            Self::Integer(_) => "Integer",
            Self::IntegerOpt(_) => "Option<Integer>",
            Self::ConfigValue(_) => "ConfigValue",
            Self::Signature(_) => "Signature",
            Self::Email(_) => "Email",
            Self::SizeHint(_) => "SizeHint",
            Self::Timestamp(_) => "Timestamp",
            Self::TimestampRange(_) => "TimestampRange",
            Self::Template(_) => "Template",
            Self::ListTemplate(_) => "ListTemplate",
        }
    }

    fn try_into_boolean(self) -> Option<BoxedTemplateProperty<'a, bool>> {
        match self {
            Self::String(property) => Some(property.map(|s| !s.is_empty()).into_dyn()),
            Self::StringList(property) => Some(property.map(|l| !l.is_empty()).into_dyn()),
            Self::Boolean(property) => Some(property),
            Self::Integer(_) => None,
            Self::IntegerOpt(property) => Some(property.map(|opt| opt.is_some()).into_dyn()),
            Self::ConfigValue(_) => None,
            Self::Signature(_) => None,
            Self::Email(property) => Some(property.map(|e| !e.0.is_empty()).into_dyn()),
            Self::SizeHint(_) => None,
            Self::Timestamp(_) => None,
            Self::TimestampRange(_) => None,
            // Template types could also be evaluated to boolean, but it's less likely
            // to apply label() or .map() and use the result as conditional. It's also
            // unclear whether ListTemplate should behave as a "list" or a "template".
            Self::Template(_) => None,
            Self::ListTemplate(_) => None,
        }
    }

    fn try_into_integer(self) -> Option<BoxedTemplateProperty<'a, i64>> {
        match self {
            Self::Integer(property) => Some(property),
            Self::IntegerOpt(property) => Some(property.try_unwrap("Integer").into_dyn()),
            _ => None,
        }
    }

    fn try_into_stringify(self) -> Option<BoxedTemplateProperty<'a, String>> {
        match self {
            Self::String(property) => Some(property),
            _ => {
                let template = self.try_into_template()?;
                Some(PlainTextFormattedProperty::new(template).into_dyn())
            }
        }
    }

    fn try_into_serialize(self) -> Option<BoxedSerializeProperty<'a>> {
        match self {
            Self::String(property) => Some(property.into_serialize()),
            Self::StringList(property) => Some(property.into_serialize()),
            Self::Boolean(property) => Some(property.into_serialize()),
            Self::Integer(property) => Some(property.into_serialize()),
            Self::IntegerOpt(property) => Some(property.into_serialize()),
            Self::ConfigValue(property) => {
                Some(property.map(config::to_serializable_value).into_serialize())
            }
            Self::Signature(property) => Some(property.into_serialize()),
            Self::Email(property) => Some(property.into_serialize()),
            Self::SizeHint(property) => Some(property.into_serialize()),
            Self::Timestamp(property) => Some(property.into_serialize()),
            Self::TimestampRange(property) => Some(property.into_serialize()),
            Self::Template(_) => None,
            Self::ListTemplate(_) => None,
        }
    }

    fn try_into_template(self) -> Option<Box<dyn Template + 'a>> {
        match self {
            Self::String(property) => Some(property.into_template()),
            Self::StringList(property) => Some(property.into_template()),
            Self::Boolean(property) => Some(property.into_template()),
            Self::Integer(property) => Some(property.into_template()),
            Self::IntegerOpt(property) => Some(property.into_template()),
            Self::ConfigValue(property) => Some(property.into_template()),
            Self::Signature(property) => Some(property.into_template()),
            Self::Email(property) => Some(property.into_template()),
            Self::SizeHint(_) => None,
            Self::Timestamp(property) => Some(property.into_template()),
            Self::TimestampRange(property) => Some(property.into_template()),
            Self::Template(template) => Some(template),
            Self::ListTemplate(template) => Some(template),
        }
    }

    fn try_into_eq(self, other: Self) -> Option<BoxedTemplateProperty<'a, bool>> {
        match (self, other) {
            (Self::String(lhs), Self::String(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| l == r).into_dyn())
            }
            (Self::String(lhs), Self::Email(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| l == r.0).into_dyn())
            }
            (Self::Boolean(lhs), Self::Boolean(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| l == r).into_dyn())
            }
            (Self::Integer(lhs), Self::Integer(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| l == r).into_dyn())
            }
            (Self::Integer(lhs), Self::IntegerOpt(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| Some(l) == r).into_dyn())
            }
            (Self::IntegerOpt(lhs), Self::Integer(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| l == Some(r)).into_dyn())
            }
            (Self::IntegerOpt(lhs), Self::IntegerOpt(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| l == r).into_dyn())
            }
            (Self::Email(lhs), Self::Email(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| l == r).into_dyn())
            }
            (Self::Email(lhs), Self::String(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| l.0 == r).into_dyn())
            }
            (Self::String(_), _) => None,
            (Self::StringList(_), _) => None,
            (Self::Boolean(_), _) => None,
            (Self::Integer(_), _) => None,
            (Self::IntegerOpt(_), _) => None,
            (Self::ConfigValue(_), _) => None,
            (Self::Signature(_), _) => None,
            (Self::Email(_), _) => None,
            (Self::SizeHint(_), _) => None,
            (Self::Timestamp(_), _) => None,
            (Self::TimestampRange(_), _) => None,
            (Self::Template(_), _) => None,
            (Self::ListTemplate(_), _) => None,
        }
    }

    fn try_into_cmp(self, other: Self) -> Option<BoxedTemplateProperty<'a, Ordering>> {
        match (self, other) {
            (Self::Integer(lhs), Self::Integer(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| l.cmp(&r)).into_dyn())
            }
            (Self::Integer(lhs), Self::IntegerOpt(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| Some(l).cmp(&r)).into_dyn())
            }
            (Self::IntegerOpt(lhs), Self::Integer(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| l.cmp(&Some(r))).into_dyn())
            }
            (Self::IntegerOpt(lhs), Self::IntegerOpt(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| l.cmp(&r)).into_dyn())
            }
            (Self::String(_), _) => None,
            (Self::StringList(_), _) => None,
            (Self::Boolean(_), _) => None,
            (Self::Integer(_), _) => None,
            (Self::IntegerOpt(_), _) => None,
            (Self::ConfigValue(_), _) => None,
            (Self::Signature(_), _) => None,
            (Self::Email(_), _) => None,
            (Self::SizeHint(_), _) => None,
            (Self::Timestamp(_), _) => None,
            (Self::TimestampRange(_), _) => None,
            (Self::Template(_), _) => None,
            (Self::ListTemplate(_), _) => None,
        }
    }
}

/// Function that translates global function call node.
// The lifetime parameter 'a could be replaced with for<'a> to keep the method
// table away from a certain lifetime. That's technically more correct, but I
// couldn't find an easy way to expand that to the core template methods, which
// are defined for L: TemplateLanguage<'a>. That's why the build fn table is
// bound to a named lifetime, and therefore can't be cached statically.
pub type TemplateBuildFunctionFn<'a, L, P> =
    fn(&L, &mut TemplateDiagnostics, &BuildContext<P>, &FunctionCallNode) -> TemplateParseResult<P>;

type BuildMethodFn<'a, L, T, P> = fn(
    &L,
    &mut TemplateDiagnostics,
    &BuildContext<P>,
    T,
    &FunctionCallNode,
) -> TemplateParseResult<P>;

/// Function that translates method call node of self type `T`.
pub type TemplateBuildMethodFn<'a, L, T, P> = BuildMethodFn<'a, L, BoxedTemplateProperty<'a, T>, P>;

/// Function that translates method call node of `Template`.
pub type BuildTemplateMethodFn<'a, L, P> = BuildMethodFn<'a, L, Box<dyn Template + 'a>, P>;

/// Function that translates method call node of `ListTemplate`.
pub type BuildListTemplateMethodFn<'a, L, P> = BuildMethodFn<'a, L, Box<dyn ListTemplate + 'a>, P>;

/// Table of functions that translate global function call node.
pub type TemplateBuildFunctionFnMap<'a, L, P = <L as TemplateLanguage<'a>>::Property> =
    HashMap<&'static str, TemplateBuildFunctionFn<'a, L, P>>;

/// Table of functions that translate method call node of self type `T`.
pub type TemplateBuildMethodFnMap<'a, L, T, P = <L as TemplateLanguage<'a>>::Property> =
    HashMap<&'static str, TemplateBuildMethodFn<'a, L, T, P>>;

/// Table of functions that translate method call node of `Template`.
pub type BuildTemplateMethodFnMap<'a, L, P = <L as TemplateLanguage<'a>>::Property> =
    HashMap<&'static str, BuildTemplateMethodFn<'a, L, P>>;

/// Table of functions that translate method call node of `ListTemplate`.
pub type BuildListTemplateMethodFnMap<'a, L, P = <L as TemplateLanguage<'a>>::Property> =
    HashMap<&'static str, BuildListTemplateMethodFn<'a, L, P>>;

/// Symbol table of functions and methods available in the core template.
pub struct CoreTemplateBuildFnTable<'a, L: ?Sized, P = <L as TemplateLanguage<'a>>::Property> {
    pub functions: TemplateBuildFunctionFnMap<'a, L, P>,
    pub string_methods: TemplateBuildMethodFnMap<'a, L, String, P>,
    pub string_list_methods: TemplateBuildMethodFnMap<'a, L, Vec<String>, P>,
    pub boolean_methods: TemplateBuildMethodFnMap<'a, L, bool, P>,
    pub integer_methods: TemplateBuildMethodFnMap<'a, L, i64, P>,
    pub config_value_methods: TemplateBuildMethodFnMap<'a, L, ConfigValue, P>,
    pub email_methods: TemplateBuildMethodFnMap<'a, L, Email, P>,
    pub signature_methods: TemplateBuildMethodFnMap<'a, L, Signature, P>,
    pub size_hint_methods: TemplateBuildMethodFnMap<'a, L, SizeHint, P>,
    pub timestamp_methods: TemplateBuildMethodFnMap<'a, L, Timestamp, P>,
    pub timestamp_range_methods: TemplateBuildMethodFnMap<'a, L, TimestampRange, P>,
    pub template_methods: BuildTemplateMethodFnMap<'a, L, P>,
    pub list_template_methods: BuildListTemplateMethodFnMap<'a, L, P>,
}

pub fn merge_fn_map<'s, F>(base: &mut HashMap<&'s str, F>, extension: HashMap<&'s str, F>) {
    for (name, function) in extension {
        if base.insert(name, function).is_some() {
            panic!("Conflicting template definitions for '{name}' function");
        }
    }
}

impl<L: ?Sized, P> CoreTemplateBuildFnTable<'_, L, P> {
    pub fn empty() -> Self {
        Self {
            functions: HashMap::new(),
            string_methods: HashMap::new(),
            string_list_methods: HashMap::new(),
            boolean_methods: HashMap::new(),
            integer_methods: HashMap::new(),
            config_value_methods: HashMap::new(),
            signature_methods: HashMap::new(),
            email_methods: HashMap::new(),
            size_hint_methods: HashMap::new(),
            timestamp_methods: HashMap::new(),
            timestamp_range_methods: HashMap::new(),
            template_methods: HashMap::new(),
            list_template_methods: HashMap::new(),
        }
    }

    pub fn merge(&mut self, other: Self) {
        let Self {
            functions,
            string_methods,
            string_list_methods,
            boolean_methods,
            integer_methods,
            config_value_methods,
            signature_methods,
            email_methods,
            size_hint_methods,
            timestamp_methods,
            timestamp_range_methods,
            template_methods,
            list_template_methods,
        } = other;

        merge_fn_map(&mut self.functions, functions);
        merge_fn_map(&mut self.string_methods, string_methods);
        merge_fn_map(&mut self.string_list_methods, string_list_methods);
        merge_fn_map(&mut self.boolean_methods, boolean_methods);
        merge_fn_map(&mut self.integer_methods, integer_methods);
        merge_fn_map(&mut self.config_value_methods, config_value_methods);
        merge_fn_map(&mut self.signature_methods, signature_methods);
        merge_fn_map(&mut self.email_methods, email_methods);
        merge_fn_map(&mut self.size_hint_methods, size_hint_methods);
        merge_fn_map(&mut self.timestamp_methods, timestamp_methods);
        merge_fn_map(&mut self.timestamp_range_methods, timestamp_range_methods);
        merge_fn_map(&mut self.template_methods, template_methods);
        merge_fn_map(&mut self.list_template_methods, list_template_methods);
    }
}

impl<'a, L> CoreTemplateBuildFnTable<'a, L, L::Property>
where
    L: TemplateLanguage<'a> + ?Sized,
{
    /// Creates new symbol table containing the builtin functions and methods.
    pub fn builtin() -> Self {
        Self {
            functions: builtin_functions(),
            string_methods: builtin_string_methods(),
            string_list_methods: builtin_formattable_list_methods(),
            boolean_methods: HashMap::new(),
            integer_methods: HashMap::new(),
            config_value_methods: builtin_config_value_methods(),
            signature_methods: builtin_signature_methods(),
            email_methods: builtin_email_methods(),
            size_hint_methods: builtin_size_hint_methods(),
            timestamp_methods: builtin_timestamp_methods(),
            timestamp_range_methods: builtin_timestamp_range_methods(),
            template_methods: HashMap::new(),
            list_template_methods: builtin_list_template_methods(),
        }
    }

    /// Translates the function call node `function` by using this symbol table.
    pub fn build_function(
        &self,
        language: &L,
        diagnostics: &mut TemplateDiagnostics,
        build_ctx: &BuildContext<L::Property>,
        function: &FunctionCallNode,
    ) -> TemplateParseResult<L::Property> {
        let table = &self.functions;
        let build = template_parser::lookup_function(table, function)?;
        build(language, diagnostics, build_ctx, function)
    }

    /// Applies the method call node `function` to the given `property` by using
    /// this symbol table.
    pub fn build_method(
        &self,
        language: &L,
        diagnostics: &mut TemplateDiagnostics,
        build_ctx: &BuildContext<L::Property>,
        property: CoreTemplatePropertyKind<'a>,
        function: &FunctionCallNode,
    ) -> TemplateParseResult<L::Property> {
        let type_name = property.type_name();
        match property {
            CoreTemplatePropertyKind::String(property) => {
                let table = &self.string_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(language, diagnostics, build_ctx, property, function)
            }
            CoreTemplatePropertyKind::StringList(property) => {
                let table = &self.string_list_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(language, diagnostics, build_ctx, property, function)
            }
            CoreTemplatePropertyKind::Boolean(property) => {
                let table = &self.boolean_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(language, diagnostics, build_ctx, property, function)
            }
            CoreTemplatePropertyKind::Integer(property) => {
                let table = &self.integer_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(language, diagnostics, build_ctx, property, function)
            }
            CoreTemplatePropertyKind::IntegerOpt(property) => {
                let type_name = "Integer";
                let table = &self.integer_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                let inner_property = property.try_unwrap(type_name).into_dyn();
                build(language, diagnostics, build_ctx, inner_property, function)
            }
            CoreTemplatePropertyKind::ConfigValue(property) => {
                let table = &self.config_value_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(language, diagnostics, build_ctx, property, function)
            }
            CoreTemplatePropertyKind::Signature(property) => {
                let table = &self.signature_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(language, diagnostics, build_ctx, property, function)
            }
            CoreTemplatePropertyKind::Email(property) => {
                let table = &self.email_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(language, diagnostics, build_ctx, property, function)
            }
            CoreTemplatePropertyKind::SizeHint(property) => {
                let table = &self.size_hint_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(language, diagnostics, build_ctx, property, function)
            }
            CoreTemplatePropertyKind::Timestamp(property) => {
                let table = &self.timestamp_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(language, diagnostics, build_ctx, property, function)
            }
            CoreTemplatePropertyKind::TimestampRange(property) => {
                let table = &self.timestamp_range_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(language, diagnostics, build_ctx, property, function)
            }
            CoreTemplatePropertyKind::Template(template) => {
                let table = &self.template_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(language, diagnostics, build_ctx, template, function)
            }
            CoreTemplatePropertyKind::ListTemplate(template) => {
                let table = &self.list_template_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(language, diagnostics, build_ctx, template, function)
            }
        }
    }
}

/// Opaque struct that represents a template value.
pub struct Expression<P> {
    property: P,
    labels: Vec<String>,
}

impl<P> Expression<P> {
    fn unlabeled(property: P) -> Self {
        let labels = vec![];
        Self { property, labels }
    }

    fn with_label(property: P, label: impl Into<String>) -> Self {
        let labels = vec![label.into()];
        Self { property, labels }
    }
}

impl<'a, P: CoreTemplatePropertyVar<'a>> Expression<P> {
    pub fn type_name(&self) -> &'static str {
        self.property.type_name()
    }

    pub fn try_into_boolean(self) -> Option<BoxedTemplateProperty<'a, bool>> {
        self.property.try_into_boolean()
    }

    pub fn try_into_integer(self) -> Option<BoxedTemplateProperty<'a, i64>> {
        self.property.try_into_integer()
    }

    pub fn try_into_stringify(self) -> Option<BoxedTemplateProperty<'a, String>> {
        self.property.try_into_stringify()
    }

    pub fn try_into_serialize(self) -> Option<BoxedSerializeProperty<'a>> {
        self.property.try_into_serialize()
    }

    pub fn try_into_template(self) -> Option<Box<dyn Template + 'a>> {
        let template = self.property.try_into_template()?;
        if self.labels.is_empty() {
            Some(template)
        } else {
            Some(Box::new(LabelTemplate::new(template, Literal(self.labels))))
        }
    }

    pub fn try_into_eq(self, other: Self) -> Option<BoxedTemplateProperty<'a, bool>> {
        self.property.try_into_eq(other.property)
    }

    pub fn try_into_cmp(self, other: Self) -> Option<BoxedTemplateProperty<'a, Ordering>> {
        self.property.try_into_cmp(other.property)
    }
}

/// Environment (locals and self) in a stack frame.
pub struct BuildContext<'i, P> {
    /// Map of functions to create `L::Property`.
    local_variables: HashMap<&'i str, &'i dyn Fn() -> P>,
    /// Function to create `L::Property` representing `self`.
    ///
    /// This could be `local_variables["self"]`, but keyword lookup shouldn't be
    /// overridden by a user-defined `self` variable.
    self_variable: &'i dyn Fn() -> P,
}

fn build_keyword<'a, L: TemplateLanguage<'a> + ?Sized>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    build_ctx: &BuildContext<L::Property>,
    name: &str,
    name_span: pest::Span<'_>,
) -> TemplateParseResult<L::Property> {
    // Keyword is a 0-ary method on the "self" property
    let self_property = (build_ctx.self_variable)();
    let function = FunctionCallNode {
        name,
        name_span,
        args: vec![],
        keyword_args: vec![],
        args_span: name_span.end_pos().span(&name_span.end_pos()),
    };
    language
        .build_method(diagnostics, build_ctx, self_property, &function)
        .map_err(|err| match err.kind() {
            TemplateParseErrorKind::NoSuchMethod { candidates, .. } => {
                let kind = TemplateParseErrorKind::NoSuchKeyword {
                    name: name.to_owned(),
                    // TODO: filter methods by arity?
                    candidates: candidates.clone(),
                };
                TemplateParseError::with_span(kind, name_span)
            }
            // Since keyword is a 0-ary method, any argument errors mean there's
            // no such keyword.
            TemplateParseErrorKind::InvalidArguments { .. } => {
                let kind = TemplateParseErrorKind::NoSuchKeyword {
                    name: name.to_owned(),
                    // TODO: might be better to phrase the error differently
                    candidates: vec![format!("self.{name}(..)")],
                };
                TemplateParseError::with_span(kind, name_span)
            }
            // The keyword function may fail with the other reasons.
            _ => err,
        })
}

fn build_unary_operation<'a, L: TemplateLanguage<'a> + ?Sized>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    build_ctx: &BuildContext<L::Property>,
    op: UnaryOp,
    arg_node: &ExpressionNode,
) -> TemplateParseResult<L::Property> {
    match op {
        UnaryOp::LogicalNot => {
            let arg = expect_boolean_expression(language, diagnostics, build_ctx, arg_node)?;
            Ok(arg.map(|v| !v).into_dyn_wrapped())
        }
        UnaryOp::Negate => {
            let arg = expect_integer_expression(language, diagnostics, build_ctx, arg_node)?;
            let out = arg.and_then(|v| {
                v.checked_neg()
                    .ok_or_else(|| TemplatePropertyError("Attempt to negate with overflow".into()))
            });
            Ok(out.into_dyn_wrapped())
        }
    }
}

fn build_binary_operation<'a, L: TemplateLanguage<'a> + ?Sized>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    build_ctx: &BuildContext<L::Property>,
    op: BinaryOp,
    lhs_node: &ExpressionNode,
    rhs_node: &ExpressionNode,
    span: pest::Span<'_>,
) -> TemplateParseResult<L::Property> {
    match op {
        BinaryOp::LogicalOr => {
            let lhs = expect_boolean_expression(language, diagnostics, build_ctx, lhs_node)?;
            let rhs = expect_boolean_expression(language, diagnostics, build_ctx, rhs_node)?;
            let out = lhs.and_then(move |l| Ok(l || rhs.extract()?));
            Ok(out.into_dyn_wrapped())
        }
        BinaryOp::LogicalAnd => {
            let lhs = expect_boolean_expression(language, diagnostics, build_ctx, lhs_node)?;
            let rhs = expect_boolean_expression(language, diagnostics, build_ctx, rhs_node)?;
            let out = lhs.and_then(move |l| Ok(l && rhs.extract()?));
            Ok(out.into_dyn_wrapped())
        }
        BinaryOp::Eq | BinaryOp::Ne => {
            let lhs = build_expression(language, diagnostics, build_ctx, lhs_node)?;
            let rhs = build_expression(language, diagnostics, build_ctx, rhs_node)?;
            let lty = lhs.type_name();
            let rty = rhs.type_name();
            let eq = lhs.try_into_eq(rhs).ok_or_else(|| {
                let message = format!("Cannot compare expressions of type `{lty}` and `{rty}`");
                TemplateParseError::expression(message, span)
            })?;
            let out = match op {
                BinaryOp::Eq => eq.into_dyn(),
                BinaryOp::Ne => eq.map(|eq| !eq).into_dyn(),
                _ => unreachable!(),
            };
            Ok(L::Property::wrap_property(out))
        }
        BinaryOp::Ge | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Lt => {
            let lhs = build_expression(language, diagnostics, build_ctx, lhs_node)?;
            let rhs = build_expression(language, diagnostics, build_ctx, rhs_node)?;
            let lty = lhs.type_name();
            let rty = rhs.type_name();
            let cmp = lhs.try_into_cmp(rhs).ok_or_else(|| {
                let message = format!("Cannot compare expressions of type `{lty}` and `{rty}`");
                TemplateParseError::expression(message, span)
            })?;
            let out = match op {
                BinaryOp::Ge => cmp.map(|ordering| ordering.is_ge()).into_dyn(),
                BinaryOp::Gt => cmp.map(|ordering| ordering.is_gt()).into_dyn(),
                BinaryOp::Le => cmp.map(|ordering| ordering.is_le()).into_dyn(),
                BinaryOp::Lt => cmp.map(|ordering| ordering.is_lt()).into_dyn(),
                _ => unreachable!(),
            };
            Ok(L::Property::wrap_property(out))
        }
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
            let lhs = expect_integer_expression(language, diagnostics, build_ctx, lhs_node)?;
            let rhs = expect_integer_expression(language, diagnostics, build_ctx, rhs_node)?;
            let build = |op: fn(i64, i64) -> Option<i64>, msg: fn(i64) -> &'static str| {
                (lhs, rhs).and_then(move |(l, r)| {
                    op(l, r).ok_or_else(|| TemplatePropertyError(msg(r).into()))
                })
            };
            let out = match op {
                BinaryOp::Add => build(i64::checked_add, |_| "Attempt to add with overflow"),
                BinaryOp::Sub => build(i64::checked_sub, |_| "Attempt to subtract with overflow"),
                BinaryOp::Mul => build(i64::checked_mul, |_| "Attempt to multiply with overflow"),
                BinaryOp::Div => build(i64::checked_div, |r| {
                    if r == 0 {
                        "Attempt to divide by zero"
                    } else {
                        "Attempt to divide with overflow"
                    }
                }),
                BinaryOp::Rem => build(i64::checked_rem, |r| {
                    if r == 0 {
                        "Attempt to divide by zero"
                    } else {
                        "Attempt to divide with overflow"
                    }
                }),
                _ => unreachable!(),
            };
            Ok(out.into_dyn_wrapped())
        }
    }
}

fn builtin_string_methods<'a, L: TemplateLanguage<'a> + ?Sized>()
-> TemplateBuildMethodFnMap<'a, L, String> {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = TemplateBuildMethodFnMap::<L, String>::new();
    map.insert(
        "len",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|s| Ok(i64::try_from(s.len())?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "contains",
        |language, diagnostics, build_ctx, self_property, function| {
            let [needle_node] = function.expect_exact_arguments()?;
            // TODO: or .try_into_string() to disable implicit type cast?
            let needle_property =
                expect_stringify_expression(language, diagnostics, build_ctx, needle_node)?;
            let out_property = (self_property, needle_property)
                .map(|(haystack, needle)| haystack.contains(&needle));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "match",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            let [needle_node] = function.expect_exact_arguments()?;
            let needle = template_parser::expect_string_pattern(needle_node)?;
            let regex = needle.to_regex();

            let out_property = self_property.and_then(move |haystack| {
                if let Some(m) = regex.find(haystack.as_bytes()) {
                    Ok(str::from_utf8(m.as_bytes())?.to_owned())
                } else {
                    // We don't have optional strings, so empty string is the
                    // right null value.
                    Ok(String::new())
                }
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "starts_with",
        |language, diagnostics, build_ctx, self_property, function| {
            let [needle_node] = function.expect_exact_arguments()?;
            let needle_property =
                expect_stringify_expression(language, diagnostics, build_ctx, needle_node)?;
            let out_property = (self_property, needle_property)
                .map(|(haystack, needle)| haystack.starts_with(&needle));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "ends_with",
        |language, diagnostics, build_ctx, self_property, function| {
            let [needle_node] = function.expect_exact_arguments()?;
            let needle_property =
                expect_stringify_expression(language, diagnostics, build_ctx, needle_node)?;
            let out_property = (self_property, needle_property)
                .map(|(haystack, needle)| haystack.ends_with(&needle));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "remove_prefix",
        |language, diagnostics, build_ctx, self_property, function| {
            let [needle_node] = function.expect_exact_arguments()?;
            let needle_property =
                expect_stringify_expression(language, diagnostics, build_ctx, needle_node)?;
            let out_property = (self_property, needle_property).map(|(haystack, needle)| {
                haystack
                    .strip_prefix(&needle)
                    .map(ToOwned::to_owned)
                    .unwrap_or(haystack)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "remove_suffix",
        |language, diagnostics, build_ctx, self_property, function| {
            let [needle_node] = function.expect_exact_arguments()?;
            let needle_property =
                expect_stringify_expression(language, diagnostics, build_ctx, needle_node)?;
            let out_property = (self_property, needle_property).map(|(haystack, needle)| {
                haystack
                    .strip_suffix(&needle)
                    .map(ToOwned::to_owned)
                    .unwrap_or(haystack)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "trim",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|s| s.trim().to_owned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "trim_start",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|s| s.trim_start().to_owned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "trim_end",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|s| s.trim_end().to_owned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "substr",
        |language, diagnostics, build_ctx, self_property, function| {
            let [start_idx, end_idx] = function.expect_exact_arguments()?;
            let start_idx_property =
                expect_isize_expression(language, diagnostics, build_ctx, start_idx)?;
            let end_idx_property =
                expect_isize_expression(language, diagnostics, build_ctx, end_idx)?;
            let out_property = (self_property, start_idx_property, end_idx_property).map(
                |(s, start_idx, end_idx)| {
                    let start_idx = string_index_to_char_boundary(&s, start_idx);
                    let end_idx = string_index_to_char_boundary(&s, end_idx);
                    s.get(start_idx..end_idx).unwrap_or_default().to_owned()
                },
            );
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "first_line",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.map(|s| s.lines().next().unwrap_or_default().to_string());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "lines",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|s| s.lines().map(|l| l.to_owned()).collect_vec());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "split",
        |language, diagnostics, build_ctx, self_property, function| {
            let ([separator_node], [limit_node]) = function.expect_arguments()?;
            let pattern = template_parser::expect_string_pattern(separator_node)?;
            let regex = pattern.to_regex();

            if let Some(limit_node) = limit_node {
                let limit_property =
                    expect_usize_expression(language, diagnostics, build_ctx, limit_node)?;
                let out_property =
                    (self_property, limit_property).and_then(move |(haystack, limit)| {
                        let haystack_bytes = haystack.as_bytes();
                        let parts: Vec<_> = regex
                            .splitn(haystack_bytes, limit)
                            .map(|part| str::from_utf8(part).map(|s| s.to_owned()))
                            .try_collect()?;
                        Ok(parts)
                    });
                Ok(out_property.into_dyn_wrapped())
            } else {
                let out_property = self_property.and_then(move |haystack| {
                    let haystack_bytes = haystack.as_bytes();
                    let parts: Vec<_> = regex
                        .split(haystack_bytes)
                        .map(|part| str::from_utf8(part).map(|s| s.to_owned()))
                        .try_collect()?;
                    Ok(parts)
                });
                Ok(out_property.into_dyn_wrapped())
            }
        },
    );
    map.insert(
        "upper",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|s| s.to_uppercase());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "lower",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|s| s.to_lowercase());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "escape_json",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|s| serde_json::to_string(&s).unwrap());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "replace",
        |language, diagnostics, build_ctx, self_property, function| {
            let ([pattern_node, replacement_node], [limit_node]) = function.expect_arguments()?;
            let pattern = template_parser::expect_string_pattern(pattern_node)?;
            let replacement_property =
                expect_stringify_expression(language, diagnostics, build_ctx, replacement_node)?;

            let regex = pattern.to_regex();

            if let Some(limit_node) = limit_node {
                let limit_property =
                    expect_usize_expression(language, diagnostics, build_ctx, limit_node)?;
                let out_property = (self_property, replacement_property, limit_property).and_then(
                    move |(haystack, replacement, limit)| {
                        if limit == 0 {
                            // We need to special-case zero because regex.replacen(_, 0, _) replaces
                            // all occurrences, and we want zero to mean no occurrences are
                            // replaced.
                            Ok(haystack)
                        } else {
                            let haystack_bytes = haystack.as_bytes();
                            let replace_bytes = replacement.as_bytes();
                            let result = regex.replacen(haystack_bytes, limit, replace_bytes);
                            Ok(str::from_utf8(&result)?.to_owned())
                        }
                    },
                );
                Ok(out_property.into_dyn_wrapped())
            } else {
                let out_property = (self_property, replacement_property).and_then(
                    move |(haystack, replacement)| {
                        let haystack_bytes = haystack.as_bytes();
                        let replace_bytes = replacement.as_bytes();
                        let result = regex.replace_all(haystack_bytes, replace_bytes);
                        Ok(str::from_utf8(&result)?.to_owned())
                    },
                );
                Ok(out_property.into_dyn_wrapped())
            }
        },
    );
    map
}

/// Clamps and aligns the given index `i` to char boundary.
///
/// Negative index counts from the end. If the index isn't at a char boundary,
/// it will be rounded towards 0 (left or right depending on the sign.)
fn string_index_to_char_boundary(s: &str, i: isize) -> usize {
    // TODO: use floor/ceil_char_boundary() if get stabilized
    let magnitude = i.unsigned_abs();
    if i < 0 {
        let p = s.len().saturating_sub(magnitude);
        (p..=s.len()).find(|&p| s.is_char_boundary(p)).unwrap()
    } else {
        let p = magnitude.min(s.len());
        (0..=p).rev().find(|&p| s.is_char_boundary(p)).unwrap()
    }
}

fn builtin_config_value_methods<'a, L: TemplateLanguage<'a> + ?Sized>()
-> TemplateBuildMethodFnMap<'a, L, ConfigValue> {
    fn extract<'de, T: Deserialize<'de>>(value: ConfigValue) -> Result<T, TemplatePropertyError> {
        T::deserialize(value.into_deserializer())
            // map to err.message() because TomlError appends newline to it
            .map_err(|err| TemplatePropertyError(err.message().into()))
    }

    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = TemplateBuildMethodFnMap::<L, ConfigValue>::new();
    // These methods are called "as_<type>", not "to_<type>" to clarify that
    // they'll never convert types (e.g. integer to string.) Since templater
    // doesn't provide binding syntax, there's no need to distinguish between
    // reference and consuming access.
    map.insert(
        "as_boolean",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(extract::<bool>);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "as_integer",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(extract::<i64>);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "as_string",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(extract::<String>);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "as_string_list",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(extract::<Vec<String>>);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    // TODO: add is_<type>() -> Boolean?
    // TODO: add .get(key) -> ConfigValue or Option<ConfigValue>?
    map
}

fn builtin_signature_methods<'a, L: TemplateLanguage<'a> + ?Sized>()
-> TemplateBuildMethodFnMap<'a, L, Signature> {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = TemplateBuildMethodFnMap::<L, Signature>::new();
    map.insert(
        "name",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|signature| signature.name);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "email",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|signature| Email(signature.email));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "timestamp",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|signature| signature.timestamp);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

fn builtin_email_methods<'a, L: TemplateLanguage<'a> + ?Sized>()
-> TemplateBuildMethodFnMap<'a, L, Email> {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = TemplateBuildMethodFnMap::<L, Email>::new();
    map.insert(
        "local",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|email| {
                let (local, _) = text_util::split_email(&email.0);
                local.to_owned()
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "domain",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|email| {
                let (_, domain) = text_util::split_email(&email.0);
                domain.unwrap_or_default().to_owned()
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

fn builtin_size_hint_methods<'a, L: TemplateLanguage<'a> + ?Sized>()
-> TemplateBuildMethodFnMap<'a, L, SizeHint> {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = TemplateBuildMethodFnMap::<L, SizeHint>::new();
    map.insert(
        "lower",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|(lower, _)| Ok(i64::try_from(lower)?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "upper",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.and_then(|(_, upper)| Ok(upper.map(i64::try_from).transpose()?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "exact",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|(lower, upper)| {
                let exact = (Some(lower) == upper).then_some(lower);
                Ok(exact.map(i64::try_from).transpose()?)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "zero",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|(_, upper)| upper == Some(0));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

fn builtin_timestamp_methods<'a, L: TemplateLanguage<'a> + ?Sized>()
-> TemplateBuildMethodFnMap<'a, L, Timestamp> {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = TemplateBuildMethodFnMap::<L, Timestamp>::new();
    map.insert(
        "ago",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let now = Timestamp::now();
            let format = timeago::Formatter::new();
            let out_property = self_property.and_then(move |timestamp| {
                Ok(time_util::format_duration(&timestamp, &now, &format)?)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "format",
        |_language, diagnostics, _build_ctx, self_property, function| {
            // No dynamic string is allowed as the templater has no runtime error type.
            let [format_node] = function.expect_exact_arguments()?;
            let format =
                template_parser::catch_aliases(diagnostics, format_node, |_diagnostics, node| {
                    let format = template_parser::expect_string_literal(node)?;
                    time_util::FormattingItems::parse(format).ok_or_else(|| {
                        TemplateParseError::expression("Invalid time format", node.span)
                    })
                })?
                .into_owned();
            let out_property = self_property.and_then(move |timestamp| {
                Ok(time_util::format_absolute_timestamp_with(
                    &timestamp, &format,
                )?)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "utc",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|mut timestamp| {
                timestamp.tz_offset = 0;
                timestamp
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "local",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let tz_offset = std::env::var("JJ_TZ_OFFSET_MINS")
                .ok()
                .and_then(|tz_string| tz_string.parse::<i32>().ok())
                .unwrap_or_else(|| chrono::Local::now().offset().local_minus_utc() / 60);
            let out_property = self_property.map(move |mut timestamp| {
                timestamp.tz_offset = tz_offset;
                timestamp
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "after",
        |_language, diagnostics, _build_ctx, self_property, function| {
            let [date_pattern_node] = function.expect_exact_arguments()?;
            let now = chrono::Local::now();
            let date_pattern = template_parser::catch_aliases(
                diagnostics,
                date_pattern_node,
                |_diagnostics, node| {
                    let date_pattern = template_parser::expect_string_literal(node)?;
                    DatePattern::from_str_kind(date_pattern, function.name, now).map_err(|err| {
                        TemplateParseError::expression("Invalid date pattern", node.span)
                            .with_source(err)
                    })
                },
            )?;
            let out_property = self_property.map(move |timestamp| date_pattern.matches(&timestamp));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert("before", map["after"]);
    map
}

fn builtin_timestamp_range_methods<'a, L: TemplateLanguage<'a> + ?Sized>()
-> TemplateBuildMethodFnMap<'a, L, TimestampRange> {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = TemplateBuildMethodFnMap::<L, TimestampRange>::new();
    map.insert(
        "start",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|time_range| time_range.start);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "end",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|time_range| time_range.end);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "duration",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            // TODO: Introduce duration type, and move formatting to it.
            let out_property = self_property.and_then(|time_range| {
                let mut f = timeago::Formatter::new();
                f.min_unit(timeago::TimeUnit::Microseconds).ago("");
                let duration = time_util::format_duration(&time_range.start, &time_range.end, &f)?;
                if duration == "now" {
                    Ok("less than a microsecond".to_owned())
                } else {
                    Ok(duration)
                }
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

fn builtin_list_template_methods<'a, L: TemplateLanguage<'a> + ?Sized>()
-> BuildListTemplateMethodFnMap<'a, L> {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = BuildListTemplateMethodFnMap::<L>::new();
    map.insert(
        "join",
        |language, diagnostics, build_ctx, self_template, function| {
            let [separator_node] = function.expect_exact_arguments()?;
            let separator =
                expect_template_expression(language, diagnostics, build_ctx, separator_node)?;
            Ok(L::Property::wrap_template(self_template.join(separator)))
        },
    );
    map
}

/// Creates new symbol table for printable list property.
pub fn builtin_formattable_list_methods<'a, L, O>() -> TemplateBuildMethodFnMap<'a, L, Vec<O>>
where
    L: TemplateLanguage<'a> + ?Sized,
    L::Property: WrapTemplateProperty<'a, O> + WrapTemplateProperty<'a, Vec<O>>,
    O: Template + Clone + 'a,
{
    let mut map = builtin_unformattable_list_methods::<L, O>();
    map.insert(
        "join",
        |language, diagnostics, build_ctx, self_property, function| {
            let [separator_node] = function.expect_exact_arguments()?;
            let separator =
                expect_template_expression(language, diagnostics, build_ctx, separator_node)?;
            let template =
                ListPropertyTemplate::new(self_property, separator, |formatter, item| {
                    item.format(formatter)
                });
            Ok(L::Property::wrap_template(Box::new(template)))
        },
    );
    map
}

/// Creates new symbol table for unprintable list property.
pub fn builtin_unformattable_list_methods<'a, L, O>() -> TemplateBuildMethodFnMap<'a, L, Vec<O>>
where
    L: TemplateLanguage<'a> + ?Sized,
    L::Property: WrapTemplateProperty<'a, O> + WrapTemplateProperty<'a, Vec<O>>,
    O: Clone + 'a,
{
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = TemplateBuildMethodFnMap::<L, Vec<O>>::new();
    map.insert(
        "len",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|items| Ok(i64::try_from(items.len())?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "filter",
        |language, diagnostics, build_ctx, self_property, function| {
            let out_property: BoxedTemplateProperty<'a, Vec<O>> =
                build_filter_operation(language, diagnostics, build_ctx, self_property, function)?;
            Ok(L::Property::wrap_property(out_property))
        },
    );
    map.insert(
        "map",
        |language, diagnostics, build_ctx, self_property, function| {
            let template =
                build_map_operation(language, diagnostics, build_ctx, self_property, function)?;
            Ok(L::Property::wrap_list_template(template))
        },
    );
    map.insert(
        "any",
        |language, diagnostics, build_ctx, self_property, function| {
            let out_property =
                build_any_operation(language, diagnostics, build_ctx, self_property, function)?;
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "all",
        |language, diagnostics, build_ctx, self_property, function| {
            let out_property =
                build_all_operation(language, diagnostics, build_ctx, self_property, function)?;
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

/// Builds expression that extracts iterable property and filters its items.
fn build_filter_operation<'a, L, O, P, B>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    build_ctx: &BuildContext<L::Property>,
    self_property: P,
    function: &FunctionCallNode,
) -> TemplateParseResult<BoxedTemplateProperty<'a, B>>
where
    L: TemplateLanguage<'a> + ?Sized,
    L::Property: WrapTemplateProperty<'a, O>,
    P: TemplateProperty + 'a,
    P::Output: IntoIterator<Item = O>,
    O: Clone + 'a,
    B: FromIterator<O>,
{
    let [lambda_node] = function.expect_exact_arguments()?;
    let item_placeholder = PropertyPlaceholder::new();
    let item_predicate =
        template_parser::catch_aliases(diagnostics, lambda_node, |diagnostics, node| {
            let lambda = template_parser::expect_lambda(node)?;
            build_lambda_expression(
                build_ctx,
                lambda,
                &[&|| item_placeholder.clone().into_dyn_wrapped()],
                |build_ctx, body| expect_boolean_expression(language, diagnostics, build_ctx, body),
            )
        })?;
    let out_property = self_property.and_then(move |items| {
        items
            .into_iter()
            .filter_map(|item| {
                // Evaluate predicate with the current item
                item_placeholder.set(item);
                let result = item_predicate.extract();
                let item = item_placeholder.take().unwrap();
                result.map(|pred| pred.then_some(item)).transpose()
            })
            .collect()
    });
    Ok(out_property.into_dyn())
}

/// Builds expression that extracts iterable property and applies template to
/// each item.
fn build_map_operation<'a, L, O, P>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    build_ctx: &BuildContext<L::Property>,
    self_property: P,
    function: &FunctionCallNode,
) -> TemplateParseResult<Box<dyn ListTemplate + 'a>>
where
    L: TemplateLanguage<'a> + ?Sized,
    L::Property: WrapTemplateProperty<'a, O>,
    P: TemplateProperty + 'a,
    P::Output: IntoIterator<Item = O>,
    O: Clone + 'a,
{
    let [lambda_node] = function.expect_exact_arguments()?;
    let item_placeholder = PropertyPlaceholder::new();
    let item_template =
        template_parser::catch_aliases(diagnostics, lambda_node, |diagnostics, node| {
            let lambda = template_parser::expect_lambda(node)?;
            build_lambda_expression(
                build_ctx,
                lambda,
                &[&|| item_placeholder.clone().into_dyn_wrapped()],
                |build_ctx, body| {
                    expect_template_expression(language, diagnostics, build_ctx, body)
                },
            )
        })?;
    let list_template = ListPropertyTemplate::new(
        self_property,
        Literal(" "), // separator
        move |formatter, item| {
            item_placeholder.with_value(item, || item_template.format(formatter))
        },
    );
    Ok(Box::new(list_template))
}

/// Builds expression that checks if any item in the list satisfies the
/// predicate.
fn build_any_operation<'a, L, O, P>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    build_ctx: &BuildContext<L::Property>,
    self_property: P,
    function: &FunctionCallNode,
) -> TemplateParseResult<BoxedTemplateProperty<'a, bool>>
where
    L: TemplateLanguage<'a> + ?Sized,
    L::Property: WrapTemplateProperty<'a, O>,
    P: TemplateProperty + 'a,
    P::Output: IntoIterator<Item = O>,
    O: Clone + 'a,
{
    let [lambda_node] = function.expect_exact_arguments()?;
    let item_placeholder = PropertyPlaceholder::new();
    let item_predicate =
        template_parser::catch_aliases(diagnostics, lambda_node, |diagnostics, node| {
            let lambda = template_parser::expect_lambda(node)?;
            build_lambda_expression(
                build_ctx,
                lambda,
                &[&|| item_placeholder.clone().into_dyn_wrapped()],
                |build_ctx, body| expect_boolean_expression(language, diagnostics, build_ctx, body),
            )
        })?;

    let out_property = self_property.and_then(move |items| {
        items
            .into_iter()
            .map(|item| item_placeholder.with_value(item, || item_predicate.extract()))
            .process_results(|mut predicates| predicates.any(|p| p))
    });
    Ok(out_property.into_dyn())
}

/// Builds expression that checks if all items in the list satisfy the
/// predicate.
fn build_all_operation<'a, L, O, P>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    build_ctx: &BuildContext<L::Property>,
    self_property: P,
    function: &FunctionCallNode,
) -> TemplateParseResult<BoxedTemplateProperty<'a, bool>>
where
    L: TemplateLanguage<'a> + ?Sized,
    L::Property: WrapTemplateProperty<'a, O>,
    P: TemplateProperty + 'a,
    P::Output: IntoIterator<Item = O>,
    O: Clone + 'a,
{
    let [lambda_node] = function.expect_exact_arguments()?;
    let item_placeholder = PropertyPlaceholder::new();
    let item_predicate =
        template_parser::catch_aliases(diagnostics, lambda_node, |diagnostics, node| {
            let lambda = template_parser::expect_lambda(node)?;
            build_lambda_expression(
                build_ctx,
                lambda,
                &[&|| item_placeholder.clone().into_dyn_wrapped()],
                |build_ctx, body| expect_boolean_expression(language, diagnostics, build_ctx, body),
            )
        })?;

    let out_property = self_property.and_then(move |items| {
        items
            .into_iter()
            .map(|item| item_placeholder.with_value(item, || item_predicate.extract()))
            .process_results(|mut predicates| predicates.all(|p| p))
    });
    Ok(out_property.into_dyn())
}

/// Builds lambda expression to be evaluated with the provided arguments.
/// `arg_fns` is usually an array of wrapped [`PropertyPlaceholder`]s.
fn build_lambda_expression<'i, P, T>(
    build_ctx: &BuildContext<'i, P>,
    lambda: &LambdaNode<'i>,
    arg_fns: &[&'i dyn Fn() -> P],
    build_body: impl FnOnce(&BuildContext<'i, P>, &ExpressionNode<'i>) -> TemplateParseResult<T>,
) -> TemplateParseResult<T> {
    if lambda.params.len() != arg_fns.len() {
        return Err(TemplateParseError::expression(
            format!("Expected {} lambda parameters", arg_fns.len()),
            lambda.params_span,
        ));
    }
    let mut local_variables = build_ctx.local_variables.clone();
    local_variables.extend(iter::zip(&lambda.params, arg_fns));
    let inner_build_ctx = BuildContext {
        local_variables,
        self_variable: build_ctx.self_variable,
    };
    build_body(&inner_build_ctx, &lambda.body)
}

fn builtin_functions<'a, L: TemplateLanguage<'a> + ?Sized>() -> TemplateBuildFunctionFnMap<'a, L> {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = TemplateBuildFunctionFnMap::<L>::new();
    map.insert("fill", |language, diagnostics, build_ctx, function| {
        let [width_node, content_node] = function.expect_exact_arguments()?;
        let width = expect_usize_expression(language, diagnostics, build_ctx, width_node)?;
        let content = expect_template_expression(language, diagnostics, build_ctx, content_node)?;
        let template =
            ReformatTemplate::new(content, move |formatter, recorded| match width.extract() {
                Ok(width) => text_util::write_wrapped(formatter.as_mut(), recorded, width),
                Err(err) => formatter.handle_error(err),
            });
        Ok(L::Property::wrap_template(Box::new(template)))
    });
    map.insert("indent", |language, diagnostics, build_ctx, function| {
        let [prefix_node, content_node] = function.expect_exact_arguments()?;
        let prefix = expect_template_expression(language, diagnostics, build_ctx, prefix_node)?;
        let content = expect_template_expression(language, diagnostics, build_ctx, content_node)?;
        let template = ReformatTemplate::new(content, move |formatter, recorded| {
            let rewrap = formatter.rewrap_fn();
            text_util::write_indented(formatter.as_mut(), recorded, |formatter| {
                prefix.format(&mut rewrap(formatter))
            })
        });
        Ok(L::Property::wrap_template(Box::new(template)))
    });
    map.insert("pad_start", |language, diagnostics, build_ctx, function| {
        let ([width_node, content_node], [fill_char_node]) =
            function.expect_named_arguments(&["", "", "fill_char"])?;
        let width = expect_usize_expression(language, diagnostics, build_ctx, width_node)?;
        let content = expect_template_expression(language, diagnostics, build_ctx, content_node)?;
        let fill_char = fill_char_node
            .map(|node| expect_template_expression(language, diagnostics, build_ctx, node))
            .transpose()?;
        let template = new_pad_template(content, fill_char, width, text_util::write_padded_start);
        Ok(L::Property::wrap_template(template))
    });
    map.insert("pad_end", |language, diagnostics, build_ctx, function| {
        let ([width_node, content_node], [fill_char_node]) =
            function.expect_named_arguments(&["", "", "fill_char"])?;
        let width = expect_usize_expression(language, diagnostics, build_ctx, width_node)?;
        let content = expect_template_expression(language, diagnostics, build_ctx, content_node)?;
        let fill_char = fill_char_node
            .map(|node| expect_template_expression(language, diagnostics, build_ctx, node))
            .transpose()?;
        let template = new_pad_template(content, fill_char, width, text_util::write_padded_end);
        Ok(L::Property::wrap_template(template))
    });
    map.insert(
        "pad_centered",
        |language, diagnostics, build_ctx, function| {
            let ([width_node, content_node], [fill_char_node]) =
                function.expect_named_arguments(&["", "", "fill_char"])?;
            let width = expect_usize_expression(language, diagnostics, build_ctx, width_node)?;
            let content =
                expect_template_expression(language, diagnostics, build_ctx, content_node)?;
            let fill_char = fill_char_node
                .map(|node| expect_template_expression(language, diagnostics, build_ctx, node))
                .transpose()?;
            let template =
                new_pad_template(content, fill_char, width, text_util::write_padded_centered);
            Ok(L::Property::wrap_template(template))
        },
    );
    map.insert(
        "truncate_start",
        |language, diagnostics, build_ctx, function| {
            let ([width_node, content_node], [ellipsis_node]) =
                function.expect_named_arguments(&["", "", "ellipsis"])?;
            let width = expect_usize_expression(language, diagnostics, build_ctx, width_node)?;
            let content =
                expect_template_expression(language, diagnostics, build_ctx, content_node)?;
            let ellipsis = ellipsis_node
                .map(|node| expect_template_expression(language, diagnostics, build_ctx, node))
                .transpose()?;
            let template =
                new_truncate_template(content, ellipsis, width, text_util::write_truncated_start);
            Ok(L::Property::wrap_template(template))
        },
    );
    map.insert(
        "truncate_end",
        |language, diagnostics, build_ctx, function| {
            let ([width_node, content_node], [ellipsis_node]) =
                function.expect_named_arguments(&["", "", "ellipsis"])?;
            let width = expect_usize_expression(language, diagnostics, build_ctx, width_node)?;
            let content =
                expect_template_expression(language, diagnostics, build_ctx, content_node)?;
            let ellipsis = ellipsis_node
                .map(|node| expect_template_expression(language, diagnostics, build_ctx, node))
                .transpose()?;
            let template =
                new_truncate_template(content, ellipsis, width, text_util::write_truncated_end);
            Ok(L::Property::wrap_template(template))
        },
    );
    map.insert("hash", |language, diagnostics, build_ctx, function| {
        let [content_node] = function.expect_exact_arguments()?;
        let content = expect_stringify_expression(language, diagnostics, build_ctx, content_node)?;
        let result = content.map(|c| hex_util::encode_hex(blake2b_hash(&c).as_ref()));
        Ok(result.into_dyn_wrapped())
    });
    map.insert("label", |language, diagnostics, build_ctx, function| {
        let [label_node, content_node] = function.expect_exact_arguments()?;
        let label_property =
            expect_stringify_expression(language, diagnostics, build_ctx, label_node)?;
        let content = expect_template_expression(language, diagnostics, build_ctx, content_node)?;
        let labels =
            label_property.map(|s| s.split_whitespace().map(ToString::to_string).collect());
        Ok(L::Property::wrap_template(Box::new(LabelTemplate::new(
            content, labels,
        ))))
    });
    map.insert(
        "raw_escape_sequence",
        |language, diagnostics, build_ctx, function| {
            let [content_node] = function.expect_exact_arguments()?;
            let content =
                expect_template_expression(language, diagnostics, build_ctx, content_node)?;
            Ok(L::Property::wrap_template(Box::new(
                RawEscapeSequenceTemplate(content),
            )))
        },
    );
    map.insert("stringify", |language, diagnostics, build_ctx, function| {
        let [content_node] = function.expect_exact_arguments()?;
        let content = expect_stringify_expression(language, diagnostics, build_ctx, content_node)?;
        Ok(L::Property::wrap_property(content))
    });
    map.insert("json", |language, diagnostics, build_ctx, function| {
        // TODO: Add pretty=true|false? or json(key=value, ..)? The latter might
        // be implemented as a map constructor/literal if we add support for
        // heterogeneous list/map types.
        let [value_node] = function.expect_exact_arguments()?;
        let value = expect_serialize_expression(language, diagnostics, build_ctx, value_node)?;
        let out_property = value.and_then(|v| Ok(serde_json::to_string(&v)?));
        Ok(out_property.into_dyn_wrapped())
    });
    map.insert("if", |language, diagnostics, build_ctx, function| {
        let ([condition_node, true_node], [false_node]) = function.expect_arguments()?;
        let condition =
            expect_boolean_expression(language, diagnostics, build_ctx, condition_node)?;
        let true_template =
            expect_template_expression(language, diagnostics, build_ctx, true_node)?;
        let false_template = false_node
            .map(|node| expect_template_expression(language, diagnostics, build_ctx, node))
            .transpose()?;
        let template = ConditionalTemplate::new(condition, true_template, false_template);
        Ok(L::Property::wrap_template(Box::new(template)))
    });
    map.insert("coalesce", |language, diagnostics, build_ctx, function| {
        let ([], content_nodes) = function.expect_some_arguments()?;
        let contents = content_nodes
            .iter()
            .map(|node| expect_template_expression(language, diagnostics, build_ctx, node))
            .try_collect()?;
        Ok(L::Property::wrap_template(Box::new(CoalesceTemplate(
            contents,
        ))))
    });
    map.insert("concat", |language, diagnostics, build_ctx, function| {
        let ([], content_nodes) = function.expect_some_arguments()?;
        let contents = content_nodes
            .iter()
            .map(|node| expect_template_expression(language, diagnostics, build_ctx, node))
            .try_collect()?;
        Ok(L::Property::wrap_template(Box::new(ConcatTemplate(
            contents,
        ))))
    });
    map.insert("join", |language, diagnostics, build_ctx, function| {
        let ([separator_node], content_nodes) = function.expect_some_arguments()?;
        let separator =
            expect_template_expression(language, diagnostics, build_ctx, separator_node)?;
        let contents = content_nodes
            .iter()
            .map(|node| expect_template_expression(language, diagnostics, build_ctx, node))
            .try_collect()?;
        Ok(L::Property::wrap_template(Box::new(JoinTemplate::new(
            separator, contents,
        ))))
    });
    map.insert("separate", |language, diagnostics, build_ctx, function| {
        let ([separator_node], content_nodes) = function.expect_some_arguments()?;
        let separator =
            expect_template_expression(language, diagnostics, build_ctx, separator_node)?;
        let contents = content_nodes
            .iter()
            .map(|node| expect_template_expression(language, diagnostics, build_ctx, node))
            .try_collect()?;
        Ok(L::Property::wrap_template(Box::new(SeparateTemplate::new(
            separator, contents,
        ))))
    });
    map.insert("surround", |language, diagnostics, build_ctx, function| {
        let [prefix_node, suffix_node, content_node] = function.expect_exact_arguments()?;
        let prefix = expect_template_expression(language, diagnostics, build_ctx, prefix_node)?;
        let suffix = expect_template_expression(language, diagnostics, build_ctx, suffix_node)?;
        let content = expect_template_expression(language, diagnostics, build_ctx, content_node)?;
        let template = ReformatTemplate::new(content, move |formatter, recorded| {
            if recorded.data().is_empty() {
                return Ok(());
            }
            prefix.format(formatter)?;
            recorded.replay(formatter.as_mut())?;
            suffix.format(formatter)?;
            Ok(())
        });
        Ok(L::Property::wrap_template(Box::new(template)))
    });
    map.insert("config", |language, diagnostics, _build_ctx, function| {
        // Dynamic lookup can be implemented if needed. The name is literal
        // string for now so the error can be reported early.
        let [name_node] = function.expect_exact_arguments()?;
        let name: ConfigNamePathBuf =
            template_parser::catch_aliases(diagnostics, name_node, |_diagnostics, node| {
                let name = template_parser::expect_string_literal(node)?;
                name.parse().map_err(|err| {
                    TemplateParseError::expression("Failed to parse config name", node.span)
                        .with_source(err)
                })
            })?;
        let value = language.settings().get_value(&name).map_err(|err| {
            TemplateParseError::expression("Failed to get config value", function.name_span)
                .with_source(err)
        })?;
        // .decorated("", "") to trim leading/trailing whitespace
        Ok(Literal(value.decorated("", "")).into_dyn_wrapped())
    });
    map
}

fn new_pad_template<'a, W>(
    content: Box<dyn Template + 'a>,
    fill_char: Option<Box<dyn Template + 'a>>,
    width: BoxedTemplateProperty<'a, usize>,
    write_padded: W,
) -> Box<dyn Template + 'a>
where
    W: Fn(&mut dyn Formatter, &FormatRecorder, &FormatRecorder, usize) -> io::Result<()> + 'a,
{
    let default_fill_char = FormatRecorder::with_data(" ");
    let template = ReformatTemplate::new(content, move |formatter, recorded| {
        let width = match width.extract() {
            Ok(width) => width,
            Err(err) => return formatter.handle_error(err),
        };
        let mut fill_char_recorder;
        let recorded_fill_char = if let Some(fill_char) = &fill_char {
            let rewrap = formatter.rewrap_fn();
            fill_char_recorder = FormatRecorder::new();
            fill_char.format(&mut rewrap(&mut fill_char_recorder))?;
            &fill_char_recorder
        } else {
            &default_fill_char
        };
        write_padded(formatter.as_mut(), recorded, recorded_fill_char, width)
    });
    Box::new(template)
}

fn new_truncate_template<'a, W>(
    content: Box<dyn Template + 'a>,
    ellipsis: Option<Box<dyn Template + 'a>>,
    width: BoxedTemplateProperty<'a, usize>,
    write_truncated: W,
) -> Box<dyn Template + 'a>
where
    W: Fn(&mut dyn Formatter, &FormatRecorder, &FormatRecorder, usize) -> io::Result<usize> + 'a,
{
    let default_ellipsis = FormatRecorder::with_data("");
    let template = ReformatTemplate::new(content, move |formatter, recorded| {
        let width = match width.extract() {
            Ok(width) => width,
            Err(err) => return formatter.handle_error(err),
        };
        let mut ellipsis_recorder;
        let recorded_ellipsis = if let Some(ellipsis) = &ellipsis {
            let rewrap = formatter.rewrap_fn();
            ellipsis_recorder = FormatRecorder::new();
            ellipsis.format(&mut rewrap(&mut ellipsis_recorder))?;
            &ellipsis_recorder
        } else {
            &default_ellipsis
        };
        write_truncated(formatter.as_mut(), recorded, recorded_ellipsis, width)?;
        Ok(())
    });
    Box::new(template)
}

/// Builds intermediate expression tree from AST nodes.
pub fn build_expression<'a, L: TemplateLanguage<'a> + ?Sized>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    build_ctx: &BuildContext<L::Property>,
    node: &ExpressionNode,
) -> TemplateParseResult<Expression<L::Property>> {
    template_parser::catch_aliases(diagnostics, node, |diagnostics, node| match &node.kind {
        ExpressionKind::Identifier(name) => {
            if let Some(make) = build_ctx.local_variables.get(name) {
                // Don't label a local variable with its name
                Ok(Expression::unlabeled(make()))
            } else if *name == "self" {
                // "self" is a special variable, so don't label it
                let make = build_ctx.self_variable;
                Ok(Expression::unlabeled(make()))
            } else {
                let property = build_keyword(language, diagnostics, build_ctx, name, node.span)
                    .map_err(|err| {
                        err.extend_keyword_candidates(itertools::chain(
                            build_ctx.local_variables.keys().copied(),
                            ["self"],
                        ))
                    })?;
                Ok(Expression::with_label(property, *name))
            }
        }
        ExpressionKind::Boolean(value) => {
            let property = Literal(*value).into_dyn_wrapped();
            Ok(Expression::unlabeled(property))
        }
        ExpressionKind::Integer(value) => {
            let property = Literal(*value).into_dyn_wrapped();
            Ok(Expression::unlabeled(property))
        }
        ExpressionKind::String(value) => {
            let property = Literal(value.clone()).into_dyn_wrapped();
            Ok(Expression::unlabeled(property))
        }
        ExpressionKind::StringPattern { .. } => Err(TemplateParseError::expression(
            "String patterns may not be used as expression values",
            node.span,
        )),
        ExpressionKind::Unary(op, arg_node) => {
            let property = build_unary_operation(language, diagnostics, build_ctx, *op, arg_node)?;
            Ok(Expression::unlabeled(property))
        }
        ExpressionKind::Binary(op, lhs_node, rhs_node) => {
            let property = build_binary_operation(
                language,
                diagnostics,
                build_ctx,
                *op,
                lhs_node,
                rhs_node,
                node.span,
            )?;
            Ok(Expression::unlabeled(property))
        }
        ExpressionKind::Concat(nodes) => {
            let templates = nodes
                .iter()
                .map(|node| expect_template_expression(language, diagnostics, build_ctx, node))
                .try_collect()?;
            let property = L::Property::wrap_template(Box::new(ConcatTemplate(templates)));
            Ok(Expression::unlabeled(property))
        }
        ExpressionKind::FunctionCall(function) => {
            let property = language.build_function(diagnostics, build_ctx, function)?;
            Ok(Expression::unlabeled(property))
        }
        ExpressionKind::MethodCall(method) => {
            let mut expression =
                build_expression(language, diagnostics, build_ctx, &method.object)?;
            expression.property = language.build_method(
                diagnostics,
                build_ctx,
                expression.property,
                &method.function,
            )?;
            expression.labels.push(method.function.name.to_owned());
            Ok(expression)
        }
        ExpressionKind::Lambda(_) => Err(TemplateParseError::expression(
            "Lambda cannot be defined here",
            node.span,
        )),
        ExpressionKind::AliasExpanded(..) => unreachable!(),
    })
}

/// Builds template evaluation tree from AST nodes, with fresh build context.
pub fn build<'a, C, L>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    node: &ExpressionNode,
) -> TemplateParseResult<TemplateRenderer<'a, C>>
where
    C: Clone + 'a,
    L: TemplateLanguage<'a> + ?Sized,
    L::Property: WrapTemplateProperty<'a, C>,
{
    let self_placeholder = PropertyPlaceholder::new();
    let build_ctx = BuildContext {
        local_variables: HashMap::new(),
        self_variable: &|| self_placeholder.clone().into_dyn_wrapped(),
    };
    let template = expect_template_expression(language, diagnostics, &build_ctx, node)?;
    Ok(TemplateRenderer::new(template, self_placeholder))
}

/// Parses text, expands aliases, then builds template evaluation tree.
pub fn parse<'a, C, L>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    template_text: &str,
    aliases_map: &TemplateAliasesMap,
) -> TemplateParseResult<TemplateRenderer<'a, C>>
where
    C: Clone + 'a,
    L: TemplateLanguage<'a> + ?Sized,
    L::Property: WrapTemplateProperty<'a, C>,
{
    let node = template_parser::parse(template_text, aliases_map)?;
    build(language, diagnostics, &node).map_err(|err| err.extend_alias_candidates(aliases_map))
}

pub fn expect_boolean_expression<'a, L: TemplateLanguage<'a> + ?Sized>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    build_ctx: &BuildContext<L::Property>,
    node: &ExpressionNode,
) -> TemplateParseResult<BoxedTemplateProperty<'a, bool>> {
    expect_expression_of_type(
        language,
        diagnostics,
        build_ctx,
        node,
        "Boolean",
        |expression| expression.try_into_boolean(),
    )
}

pub fn expect_integer_expression<'a, L: TemplateLanguage<'a> + ?Sized>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    build_ctx: &BuildContext<L::Property>,
    node: &ExpressionNode,
) -> TemplateParseResult<BoxedTemplateProperty<'a, i64>> {
    expect_expression_of_type(
        language,
        diagnostics,
        build_ctx,
        node,
        "Integer",
        |expression| expression.try_into_integer(),
    )
}

/// If the given expression `node` is of `Integer` type, converts it to `isize`.
pub fn expect_isize_expression<'a, L: TemplateLanguage<'a> + ?Sized>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    build_ctx: &BuildContext<L::Property>,
    node: &ExpressionNode,
) -> TemplateParseResult<BoxedTemplateProperty<'a, isize>> {
    let i64_property = expect_integer_expression(language, diagnostics, build_ctx, node)?;
    let isize_property = i64_property.and_then(|v| Ok(isize::try_from(v)?));
    Ok(isize_property.into_dyn())
}

/// If the given expression `node` is of `Integer` type, converts it to `usize`.
pub fn expect_usize_expression<'a, L: TemplateLanguage<'a> + ?Sized>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    build_ctx: &BuildContext<L::Property>,
    node: &ExpressionNode,
) -> TemplateParseResult<BoxedTemplateProperty<'a, usize>> {
    let i64_property = expect_integer_expression(language, diagnostics, build_ctx, node)?;
    let usize_property = i64_property.and_then(|v| Ok(usize::try_from(v)?));
    Ok(usize_property.into_dyn())
}

pub fn expect_stringify_expression<'a, L: TemplateLanguage<'a> + ?Sized>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    build_ctx: &BuildContext<L::Property>,
    node: &ExpressionNode,
) -> TemplateParseResult<BoxedTemplateProperty<'a, String>> {
    // Since any formattable type can be converted to a string property, the
    // expected type is not a String.
    expect_expression_of_type(
        language,
        diagnostics,
        build_ctx,
        node,
        "Stringify",
        |expression| expression.try_into_stringify(),
    )
}

pub fn expect_serialize_expression<'a, L: TemplateLanguage<'a> + ?Sized>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    build_ctx: &BuildContext<L::Property>,
    node: &ExpressionNode,
) -> TemplateParseResult<BoxedSerializeProperty<'a>> {
    expect_expression_of_type(
        language,
        diagnostics,
        build_ctx,
        node,
        "Serialize",
        |expression| expression.try_into_serialize(),
    )
}

pub fn expect_template_expression<'a, L: TemplateLanguage<'a> + ?Sized>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    build_ctx: &BuildContext<L::Property>,
    node: &ExpressionNode,
) -> TemplateParseResult<Box<dyn Template + 'a>> {
    expect_expression_of_type(
        language,
        diagnostics,
        build_ctx,
        node,
        "Template",
        |expression| expression.try_into_template(),
    )
}

fn expect_expression_of_type<'a, L: TemplateLanguage<'a> + ?Sized, T>(
    language: &L,
    diagnostics: &mut TemplateDiagnostics,
    build_ctx: &BuildContext<L::Property>,
    node: &ExpressionNode,
    expected_type: &str,
    f: impl FnOnce(Expression<L::Property>) -> Option<T>,
) -> TemplateParseResult<T> {
    template_parser::catch_aliases(diagnostics, node, |diagnostics, node| {
        let expression = build_expression(language, diagnostics, build_ctx, node)?;
        let actual_type = expression.type_name();
        f(expression)
            .ok_or_else(|| TemplateParseError::expected_type(expected_type, actual_type, node.span))
    })
}

#[cfg(test)]
mod tests {
    use jj_lib::backend::MillisSinceEpoch;
    use jj_lib::config::StackedConfig;

    use super::*;
    use crate::formatter;
    use crate::formatter::ColorFormatter;
    use crate::generic_templater;
    use crate::generic_templater::GenericTemplateLanguage;

    #[derive(Clone, Debug, serde::Serialize)]
    struct Context;

    type TestTemplateLanguage = GenericTemplateLanguage<'static, Context>;
    type TestTemplatePropertyKind = <TestTemplateLanguage as TemplateLanguage<'static>>::Property;

    generic_templater::impl_self_property_wrapper!(Context);

    /// Helper to set up template evaluation environment.
    struct TestTemplateEnv {
        language: TestTemplateLanguage,
        aliases_map: TemplateAliasesMap,
        color_rules: Vec<(Vec<String>, formatter::Style)>,
    }

    impl TestTemplateEnv {
        fn new() -> Self {
            Self::with_config(StackedConfig::with_defaults())
        }

        fn with_config(config: StackedConfig) -> Self {
            let settings = UserSettings::from_config(config).unwrap();
            Self {
                language: TestTemplateLanguage::new(&settings),
                aliases_map: TemplateAliasesMap::new(),
                color_rules: Vec::new(),
            }
        }
    }

    impl TestTemplateEnv {
        fn add_keyword<F>(&mut self, name: &'static str, build: F)
        where
            F: Fn() -> TestTemplatePropertyKind + 'static,
        {
            self.language.add_keyword(name, move |_| Ok(build()));
        }

        fn add_alias(&mut self, decl: impl AsRef<str>, defn: impl Into<String>) {
            self.aliases_map.insert(decl, defn).unwrap();
        }

        fn add_color(&mut self, label: &str, fg: crossterm::style::Color) {
            let labels = label.split_whitespace().map(|s| s.to_owned()).collect();
            let style = formatter::Style {
                fg: Some(fg),
                ..Default::default()
            };
            self.color_rules.push((labels, style));
        }

        fn parse(&self, template: &str) -> TemplateParseResult<TemplateRenderer<'static, Context>> {
            parse(
                &self.language,
                &mut TemplateDiagnostics::new(),
                template,
                &self.aliases_map,
            )
        }

        fn parse_err(&self, template: &str) -> String {
            let err = self
                .parse(template)
                .err()
                .expect("Got unexpected successful template rendering");

            iter::successors(Some(&err as &dyn std::error::Error), |e| e.source()).join("\n")
        }

        fn render_ok(&self, template: &str) -> String {
            let template = self.parse(template).unwrap();
            let mut output = Vec::new();
            let mut formatter =
                ColorFormatter::new(&mut output, self.color_rules.clone().into(), false);
            template.format(&Context, &mut formatter).unwrap();
            drop(formatter);
            String::from_utf8(output).unwrap()
        }
    }

    fn literal<'a, O>(value: O) -> TestTemplatePropertyKind
    where
        O: Clone + 'a,
        TestTemplatePropertyKind: WrapTemplateProperty<'a, O>,
    {
        Literal(value).into_dyn_wrapped()
    }

    fn new_error_property<'a, O>(message: &'a str) -> TestTemplatePropertyKind
    where
        TestTemplatePropertyKind: WrapTemplateProperty<'a, O>,
    {
        Literal(())
            .and_then(|()| Err(TemplatePropertyError(message.into())))
            .into_dyn_wrapped()
    }

    fn new_signature(name: &str, email: &str) -> Signature {
        Signature {
            name: name.to_owned(),
            email: email.to_owned(),
            timestamp: new_timestamp(0, 0),
        }
    }

    fn new_timestamp(msec: i64, tz_offset: i32) -> Timestamp {
        Timestamp {
            timestamp: MillisSinceEpoch(msec),
            tz_offset,
        }
    }

    #[test]
    fn test_parsed_tree() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("divergent", || literal(false));
        env.add_keyword("empty", || literal(true));
        env.add_keyword("hello", || literal("Hello".to_owned()));

        // Empty
        insta::assert_snapshot!(env.render_ok(r#"  "#), @"");

        // Single term with whitespace
        insta::assert_snapshot!(env.render_ok(r#"  hello.upper()  "#), @"HELLO");

        // Multiple terms
        insta::assert_snapshot!(env.render_ok(r#"  hello.upper()  ++ true "#), @"HELLOtrue");

        // Parenthesized single term
        insta::assert_snapshot!(env.render_ok(r#"(hello.upper())"#), @"HELLO");

        // Parenthesized multiple terms and concatenation
        insta::assert_snapshot!(env.render_ok(r#"(hello.upper() ++ " ") ++ empty"#), @"HELLO true");

        // Parenthesized "if" condition
        insta::assert_snapshot!(env.render_ok(r#"if((divergent), "t", "f")"#), @"f");

        // Parenthesized method chaining
        insta::assert_snapshot!(env.render_ok(r#"(hello).upper()"#), @"HELLO");

        // Multi-line method chaining
        insta::assert_snapshot!(env.render_ok("hello\n  .upper()"), @"HELLO");
    }

    #[test]
    fn test_parse_error() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("description", || literal("".to_owned()));
        env.add_keyword("empty", || literal(true));

        insta::assert_snapshot!(env.parse_err(r#"foo bar"#), @r"
         --> 1:5
          |
        1 | foo bar
          |     ^---
          |
          = expected <EOI>, `++`, `||`, `&&`, `==`, `!=`, `>=`, `>`, `<=`, `<`, `+`, `-`, `*`, `/`, or `%`
        ");

        insta::assert_snapshot!(env.parse_err(r#"foo"#), @r"
         --> 1:1
          |
        1 | foo
          | ^-^
          |
          = Keyword `foo` doesn't exist
        ");

        insta::assert_snapshot!(env.parse_err(r#"foo()"#), @r"
         --> 1:1
          |
        1 | foo()
          | ^-^
          |
          = Function `foo` doesn't exist
        ");
        insta::assert_snapshot!(env.parse_err(r#"false()"#), @r"
         --> 1:1
          |
        1 | false()
          | ^---^
          |
          = Expected identifier
        ");

        insta::assert_snapshot!(env.parse_err(r#"!foo"#), @r"
         --> 1:2
          |
        1 | !foo
          |  ^-^
          |
          = Keyword `foo` doesn't exist
        ");
        insta::assert_snapshot!(env.parse_err(r#"true && 123"#), @r"
         --> 1:9
          |
        1 | true && 123
          |         ^-^
          |
          = Expected expression of type `Boolean`, but actual type is `Integer`
        ");
        insta::assert_snapshot!(env.parse_err(r#"true == 1"#), @r"
         --> 1:1
          |
        1 | true == 1
          | ^-------^
          |
          = Cannot compare expressions of type `Boolean` and `Integer`
        ");
        insta::assert_snapshot!(env.parse_err(r#"true != 'a'"#), @r"
         --> 1:1
          |
        1 | true != 'a'
          | ^---------^
          |
          = Cannot compare expressions of type `Boolean` and `String`
        ");
        insta::assert_snapshot!(env.parse_err(r#"1 == true"#), @r"
         --> 1:1
          |
        1 | 1 == true
          | ^-------^
          |
          = Cannot compare expressions of type `Integer` and `Boolean`
        ");
        insta::assert_snapshot!(env.parse_err(r#"1 != 'a'"#), @r"
         --> 1:1
          |
        1 | 1 != 'a'
          | ^------^
          |
          = Cannot compare expressions of type `Integer` and `String`
        ");
        insta::assert_snapshot!(env.parse_err(r#"'a' == true"#), @r"
         --> 1:1
          |
        1 | 'a' == true
          | ^---------^
          |
          = Cannot compare expressions of type `String` and `Boolean`
        ");
        insta::assert_snapshot!(env.parse_err(r#"'a' != 1"#), @r"
         --> 1:1
          |
        1 | 'a' != 1
          | ^------^
          |
          = Cannot compare expressions of type `String` and `Integer`
        ");
        insta::assert_snapshot!(env.parse_err(r#"'a' == label("", "")"#), @r#"
         --> 1:1
          |
        1 | 'a' == label("", "")
          | ^------------------^
          |
          = Cannot compare expressions of type `String` and `Template`
        "#);
        insta::assert_snapshot!(env.parse_err(r#"'a' > 1"#), @r"
         --> 1:1
          |
        1 | 'a' > 1
          | ^-----^
          |
          = Cannot compare expressions of type `String` and `Integer`
        ");

        insta::assert_snapshot!(env.parse_err(r#"description.first_line().foo()"#), @r"
         --> 1:26
          |
        1 | description.first_line().foo()
          |                          ^-^
          |
          = Method `foo` doesn't exist for type `String`
        ");

        insta::assert_snapshot!(env.parse_err(r#"10000000000000000000"#), @r"
         --> 1:1
          |
        1 | 10000000000000000000
          | ^------------------^
          |
          = Invalid integer literal
        number too large to fit in target type
        ");
        insta::assert_snapshot!(env.parse_err(r#"42.foo()"#), @r"
         --> 1:4
          |
        1 | 42.foo()
          |    ^-^
          |
          = Method `foo` doesn't exist for type `Integer`
        ");
        insta::assert_snapshot!(env.parse_err(r#"(-empty)"#), @r"
         --> 1:3
          |
        1 | (-empty)
          |   ^---^
          |
          = Expected expression of type `Integer`, but actual type is `Boolean`
        ");

        insta::assert_snapshot!(env.parse_err(r#"("foo" ++ "bar").baz()"#), @r#"
         --> 1:18
          |
        1 | ("foo" ++ "bar").baz()
          |                  ^-^
          |
          = Method `baz` doesn't exist for type `Template`
        "#);

        insta::assert_snapshot!(env.parse_err(r#"description.contains()"#), @r"
         --> 1:22
          |
        1 | description.contains()
          |                      ^
          |
          = Function `contains`: Expected 1 arguments
        ");

        insta::assert_snapshot!(env.parse_err(r#"description.first_line("foo")"#), @r#"
         --> 1:24
          |
        1 | description.first_line("foo")
          |                        ^---^
          |
          = Function `first_line`: Expected 0 arguments
        "#);

        insta::assert_snapshot!(env.parse_err(r#"label()"#), @r"
         --> 1:7
          |
        1 | label()
          |       ^
          |
          = Function `label`: Expected 2 arguments
        ");
        insta::assert_snapshot!(env.parse_err(r#"label("foo", "bar", "baz")"#), @r#"
         --> 1:7
          |
        1 | label("foo", "bar", "baz")
          |       ^-----------------^
          |
          = Function `label`: Expected 2 arguments
        "#);

        insta::assert_snapshot!(env.parse_err(r#"if()"#), @r"
         --> 1:4
          |
        1 | if()
          |    ^
          |
          = Function `if`: Expected 2 to 3 arguments
        ");
        insta::assert_snapshot!(env.parse_err(r#"if("foo", "bar", "baz", "quux")"#), @r#"
         --> 1:4
          |
        1 | if("foo", "bar", "baz", "quux")
          |    ^-------------------------^
          |
          = Function `if`: Expected 2 to 3 arguments
        "#);

        insta::assert_snapshot!(env.parse_err(r#"pad_start("foo", fill_char = "bar", "baz")"#), @r#"
         --> 1:37
          |
        1 | pad_start("foo", fill_char = "bar", "baz")
          |                                     ^---^
          |
          = Function `pad_start`: Positional argument follows keyword argument
        "#);

        insta::assert_snapshot!(env.parse_err(r#"if(label("foo", "bar"), "baz")"#), @r#"
         --> 1:4
          |
        1 | if(label("foo", "bar"), "baz")
          |    ^-----------------^
          |
          = Expected expression of type `Boolean`, but actual type is `Template`
        "#);

        insta::assert_snapshot!(env.parse_err(r#"|x| description"#), @r"
         --> 1:1
          |
        1 | |x| description
          | ^-------------^
          |
          = Lambda cannot be defined here
        ");
    }

    #[test]
    fn test_self_keyword() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("say_hello", || literal("Hello".to_owned()));

        insta::assert_snapshot!(env.render_ok(r#"self.say_hello()"#), @"Hello");
        insta::assert_snapshot!(env.parse_err(r#"self"#), @r"
         --> 1:1
          |
        1 | self
          | ^--^
          |
          = Expected expression of type `Template`, but actual type is `Self`
        ");
    }

    #[test]
    fn test_boolean_cast() {
        let mut env = TestTemplateEnv::new();

        insta::assert_snapshot!(env.render_ok(r#"if("", true, false)"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"if("a", true, false)"#), @"true");

        env.add_keyword("sl0", || literal::<Vec<String>>(vec![]));
        env.add_keyword("sl1", || literal(vec!["".to_owned()]));
        insta::assert_snapshot!(env.render_ok(r#"if(sl0, true, false)"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"if(sl1, true, false)"#), @"true");

        // No implicit cast of integer
        insta::assert_snapshot!(env.parse_err(r#"if(0, true, false)"#), @r"
         --> 1:4
          |
        1 | if(0, true, false)
          |    ^
          |
          = Expected expression of type `Boolean`, but actual type is `Integer`
        ");

        // Optional integer can be converted to boolean, and Some(0) is truthy.
        env.add_keyword("none_i64", || literal(None));
        env.add_keyword("some_i64", || literal(Some(0)));
        insta::assert_snapshot!(env.render_ok(r#"if(none_i64, true, false)"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"if(some_i64, true, false)"#), @"true");

        insta::assert_snapshot!(env.parse_err(r#"if(label("", ""), true, false)"#), @r#"
         --> 1:4
          |
        1 | if(label("", ""), true, false)
          |    ^-----------^
          |
          = Expected expression of type `Boolean`, but actual type is `Template`
        "#);
        insta::assert_snapshot!(env.parse_err(r#"if(sl0.map(|x| x), true, false)"#), @r"
         --> 1:4
          |
        1 | if(sl0.map(|x| x), true, false)
          |    ^------------^
          |
          = Expected expression of type `Boolean`, but actual type is `ListTemplate`
        ");

        env.add_keyword("empty_email", || literal(Email("".to_owned())));
        env.add_keyword("nonempty_email", || {
            literal(Email("local@domain".to_owned()))
        });
        insta::assert_snapshot!(env.render_ok(r#"if(empty_email, true, false)"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"if(nonempty_email, true, false)"#), @"true");
    }

    #[test]
    fn test_arithmetic_operation() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("none_i64", || literal(None));
        env.add_keyword("some_i64", || literal(Some(1)));
        env.add_keyword("i64_min", || literal(i64::MIN));
        env.add_keyword("i64_max", || literal(i64::MAX));

        insta::assert_snapshot!(env.render_ok(r#"-1"#), @"-1");
        insta::assert_snapshot!(env.render_ok(r#"--2"#), @"2");
        insta::assert_snapshot!(env.render_ok(r#"-(3)"#), @"-3");
        insta::assert_snapshot!(env.render_ok(r#"1 + 2"#), @"3");
        insta::assert_snapshot!(env.render_ok(r#"2 * 3"#), @"6");
        insta::assert_snapshot!(env.render_ok(r#"1 + 2 * 3"#), @"7");
        insta::assert_snapshot!(env.render_ok(r#"4 / 2"#), @"2");
        insta::assert_snapshot!(env.render_ok(r#"5 / 2"#), @"2");
        insta::assert_snapshot!(env.render_ok(r#"5 % 2"#), @"1");

        // Since methods of the contained value can be invoked, it makes sense
        // to apply operators to optional integers as well.
        insta::assert_snapshot!(env.render_ok(r#"-none_i64"#), @"<Error: No Integer available>");
        insta::assert_snapshot!(env.render_ok(r#"-some_i64"#), @"-1");
        insta::assert_snapshot!(env.render_ok(r#"some_i64 + some_i64"#), @"2");
        insta::assert_snapshot!(env.render_ok(r#"some_i64 + none_i64"#), @"<Error: No Integer available>");
        insta::assert_snapshot!(env.render_ok(r#"none_i64 + some_i64"#), @"<Error: No Integer available>");
        insta::assert_snapshot!(env.render_ok(r#"none_i64 + none_i64"#), @"<Error: No Integer available>");

        // No panic on integer overflow.
        insta::assert_snapshot!(
            env.render_ok(r#"-i64_min"#),
            @"<Error: Attempt to negate with overflow>");
        insta::assert_snapshot!(
            env.render_ok(r#"i64_max + 1"#),
            @"<Error: Attempt to add with overflow>");
        insta::assert_snapshot!(
            env.render_ok(r#"i64_min - 1"#),
            @"<Error: Attempt to subtract with overflow>");
        insta::assert_snapshot!(
            env.render_ok(r#"i64_max * 2"#),
            @"<Error: Attempt to multiply with overflow>");
        insta::assert_snapshot!(
            env.render_ok(r#"i64_min / -1"#),
            @"<Error: Attempt to divide with overflow>");
        insta::assert_snapshot!(
            env.render_ok(r#"1 / 0"#),
            @"<Error: Attempt to divide by zero>");
        insta::assert_snapshot!(
            env.render_ok(r#"1 % 0"#),
            @"<Error: Attempt to divide by zero>");
    }

    #[test]
    fn test_relational_operation() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("none_i64", || literal(None::<i64>));
        env.add_keyword("some_i64_0", || literal(Some(0_i64)));
        env.add_keyword("some_i64_1", || literal(Some(1_i64)));

        insta::assert_snapshot!(env.render_ok(r#"1 >= 1"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"0 >= 1"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"2 > 1"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"1 > 1"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"1 <= 1"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"2 <= 1"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"0 < 1"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"1 < 1"#), @"false");

        // none < some
        insta::assert_snapshot!(env.render_ok(r#"none_i64 < some_i64_0"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"some_i64_0 > some_i64_1"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"none_i64 < 0"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"1 > some_i64_0"#), @"true");
    }

    #[test]
    fn test_logical_operation() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("none_i64", || literal::<Option<i64>>(None));
        env.add_keyword("some_i64_0", || literal(Some(0_i64)));
        env.add_keyword("some_i64_1", || literal(Some(1_i64)));
        env.add_keyword("email1", || literal(Email("local-1@domain".to_owned())));
        env.add_keyword("email2", || literal(Email("local-2@domain".to_owned())));

        insta::assert_snapshot!(env.render_ok(r#"!false"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"false || !false"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"false && true"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"true == true"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"true == false"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"true != true"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"true != false"#), @"true");

        insta::assert_snapshot!(env.render_ok(r#"1 == 1"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"1 == 2"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"1 != 1"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"1 != 2"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"none_i64 == none_i64"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"some_i64_0 != some_i64_0"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"none_i64 == 0"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"some_i64_0 != 0"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"1 == some_i64_1"#), @"true");

        insta::assert_snapshot!(env.render_ok(r#"'a' == 'a'"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"'a' == 'b'"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"'a' != 'a'"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"'a' != 'b'"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"email1 == email1"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"email1 == email2"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"email1 == 'local-1@domain'"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"email1 != 'local-2@domain'"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"'local-1@domain' == email1"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#"'local-2@domain' != email1"#), @"true");

        insta::assert_snapshot!(env.render_ok(r#" !"" "#), @"true");
        insta::assert_snapshot!(env.render_ok(r#" "" || "a".lines() "#), @"true");

        // Short-circuiting
        env.add_keyword("bad_bool", || new_error_property::<bool>("Bad"));
        insta::assert_snapshot!(env.render_ok(r#"false && bad_bool"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"true && bad_bool"#), @"<Error: Bad>");
        insta::assert_snapshot!(env.render_ok(r#"false || bad_bool"#), @"<Error: Bad>");
        insta::assert_snapshot!(env.render_ok(r#"true || bad_bool"#), @"true");
    }

    #[test]
    fn test_list_method() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("empty", || literal(true));
        env.add_keyword("sep", || literal("sep".to_owned()));

        insta::assert_snapshot!(env.render_ok(r#""".lines().len()"#), @"0");
        insta::assert_snapshot!(env.render_ok(r#""a\nb\nc".lines().len()"#), @"3");

        insta::assert_snapshot!(env.render_ok(r#""".lines().join("|")"#), @"");
        insta::assert_snapshot!(env.render_ok(r#""a\nb\nc".lines().join("|")"#), @"a|b|c");
        // Null separator
        insta::assert_snapshot!(env.render_ok(r#""a\nb\nc".lines().join("\0")"#), @"a\0b\0c");
        // Keyword as separator
        insta::assert_snapshot!(
            env.render_ok(r#""a\nb\nc".lines().join(sep.upper())"#),
            @"aSEPbSEPc");

        insta::assert_snapshot!(
            env.render_ok(r#""a\nbb\nc".lines().filter(|s| s.len() == 1)"#),
            @"a c");

        insta::assert_snapshot!(
            env.render_ok(r#""a\nb\nc".lines().map(|s| s ++ s)"#),
            @"aa bb cc");

        // Test any() method
        insta::assert_snapshot!(
            env.render_ok(r#""a\nb\nc".lines().any(|s| s == "b")"#),
            @"true");
        insta::assert_snapshot!(
            env.render_ok(r#""a\nb\nc".lines().any(|s| s == "d")"#),
            @"false");
        insta::assert_snapshot!(
            env.render_ok(r#""".lines().any(|s| s == "a")"#),
            @"false");
        // any() with more complex predicate
        insta::assert_snapshot!(
            env.render_ok(r#""ax\nbb\nc".lines().any(|s| s.contains("x"))"#),
            @"true");
        insta::assert_snapshot!(
            env.render_ok(r#""a\nbb\nc".lines().any(|s| s.len() > 1)"#),
            @"true");

        // Test all() method
        insta::assert_snapshot!(
            env.render_ok(r#""a\nb\nc".lines().all(|s| s.len() == 1)"#),
            @"true");
        insta::assert_snapshot!(
            env.render_ok(r#""a\nbb\nc".lines().all(|s| s.len() == 1)"#),
            @"false");
        // Empty list returns true for all()
        insta::assert_snapshot!(
            env.render_ok(r#""".lines().all(|s| s == "a")"#),
            @"true");
        // all() with more complex predicate
        insta::assert_snapshot!(
            env.render_ok(r#""ax\nbx\ncx".lines().all(|s| s.ends_with("x"))"#),
            @"true");
        insta::assert_snapshot!(
            env.render_ok(r#""a\nbb\nc".lines().all(|s| s.len() < 3)"#),
            @"true");

        // Combining any/all with filter
        insta::assert_snapshot!(
            env.render_ok(r#""a\nbb\nccc".lines().filter(|s| s.len() > 1).any(|s| s == "bb")"#),
            @"true");
        insta::assert_snapshot!(
            env.render_ok(r#""a\nbb\nccc".lines().filter(|s| s.len() > 1).all(|s| s.len() >= 2)"#),
            @"true");

        // Nested any/all operations
        insta::assert_snapshot!(
            env.render_ok(r#"if("a\nb".lines().any(|s| s == "a"), "found", "not found")"#),
            @"found");
        insta::assert_snapshot!(
            env.render_ok(r#"if("a\nb".lines().all(|s| s.len() == 1), "all single", "not all")"#),
            @"all single");

        // Global keyword in item template
        insta::assert_snapshot!(
            env.render_ok(r#""a\nb\nc".lines().map(|s| s ++ empty)"#),
            @"atrue btrue ctrue");
        // Global keyword in item template shadowing 'self'
        insta::assert_snapshot!(
            env.render_ok(r#""a\nb\nc".lines().map(|self| self ++ empty)"#),
            @"atrue btrue ctrue");
        // Override global keyword 'empty'
        insta::assert_snapshot!(
            env.render_ok(r#""a\nb\nc".lines().map(|empty| empty)"#),
            @"a b c");
        // Nested map operations
        insta::assert_snapshot!(
            env.render_ok(r#""a\nb\nc".lines().map(|s| "x\ny".lines().map(|t| s ++ t))"#),
            @"ax ay bx by cx cy");
        // Nested map/join operations
        insta::assert_snapshot!(
            env.render_ok(r#""a\nb\nc".lines().map(|s| "x\ny".lines().map(|t| s ++ t).join(",")).join(";")"#),
            @"ax,ay;bx,by;cx,cy");
        // Nested string operations
        insta::assert_snapshot!(
            env.render_ok(r#""!  a\n!b\nc\n   end".remove_suffix("end").trim_end().lines().map(|s| s.remove_prefix("!").trim_start())"#),
            @"a b c");

        // Lambda expression in alias
        env.add_alias("identity", "|x| x");
        insta::assert_snapshot!(env.render_ok(r#""a\nb\nc".lines().map(identity)"#), @"a b c");

        // Not a lambda expression
        insta::assert_snapshot!(env.parse_err(r#""a".lines().map(empty)"#), @r#"
         --> 1:17
          |
        1 | "a".lines().map(empty)
          |                 ^---^
          |
          = Expected lambda expression
        "#);
        // Bad lambda parameter count
        insta::assert_snapshot!(env.parse_err(r#""a".lines().map(|| "")"#), @r#"
         --> 1:18
          |
        1 | "a".lines().map(|| "")
          |                  ^
          |
          = Expected 1 lambda parameters
        "#);
        insta::assert_snapshot!(env.parse_err(r#""a".lines().map(|a, b| "")"#), @r#"
         --> 1:18
          |
        1 | "a".lines().map(|a, b| "")
          |                  ^--^
          |
          = Expected 1 lambda parameters
        "#);
        // Bad lambda output
        insta::assert_snapshot!(env.parse_err(r#""a".lines().filter(|s| s ++ "\n")"#), @r#"
         --> 1:24
          |
        1 | "a".lines().filter(|s| s ++ "\n")
          |                        ^-------^
          |
          = Expected expression of type `Boolean`, but actual type is `Template`
        "#);

        // Error in any() and all()
        insta::assert_snapshot!(env.parse_err(r#""a".lines().any(|s| s.len())"#), @r#"
         --> 1:21
          |
        1 | "a".lines().any(|s| s.len())
          |                     ^-----^
          |
          = Expected expression of type `Boolean`, but actual type is `Integer`
        "#);
        // Bad lambda output for all()
        insta::assert_snapshot!(env.parse_err(r#""a".lines().all(|s| s ++ "x")"#), @r#"
         --> 1:21
          |
        1 | "a".lines().all(|s| s ++ "x")
          |                     ^------^
          |
          = Expected expression of type `Boolean`, but actual type is `Template`
        "#);
        // Wrong parameter count for any()
        insta::assert_snapshot!(env.parse_err(r#""a".lines().any(|| true)"#), @r#"
         --> 1:18
          |
        1 | "a".lines().any(|| true)
          |                  ^
          |
          = Expected 1 lambda parameters
        "#);
        // Wrong parameter count for all()
        insta::assert_snapshot!(env.parse_err(r#""a".lines().all(|a, b| true)"#), @r#"
         --> 1:18
          |
        1 | "a".lines().all(|a, b| true)
          |                  ^--^
          |
          = Expected 1 lambda parameters
        "#);
        // Error in lambda expression
        insta::assert_snapshot!(env.parse_err(r#""a".lines().map(|s| s.unknown())"#), @r#"
         --> 1:23
          |
        1 | "a".lines().map(|s| s.unknown())
          |                       ^-----^
          |
          = Method `unknown` doesn't exist for type `String`
        "#);
        // Error in lambda alias
        env.add_alias("too_many_params", "|x, y| x");
        insta::assert_snapshot!(env.parse_err(r#""a".lines().map(too_many_params)"#), @r#"
         --> 1:17
          |
        1 | "a".lines().map(too_many_params)
          |                 ^-------------^
          |
          = In alias `too_many_params`
         --> 1:2
          |
        1 | |x, y| x
          |  ^--^
          |
          = Expected 1 lambda parameters
        "#);
    }

    #[test]
    fn test_string_method() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("description", || literal("description 1".to_owned()));
        env.add_keyword("bad_string", || new_error_property::<String>("Bad"));

        insta::assert_snapshot!(env.render_ok(r#""".len()"#), @"0");
        insta::assert_snapshot!(env.render_ok(r#""foo".len()"#), @"3");
        insta::assert_snapshot!(env.render_ok(r#""💩".len()"#), @"4");

        insta::assert_snapshot!(env.render_ok(r#""fooo".contains("foo")"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#""foo".contains("fooo")"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#"description.contains("description")"#), @"true");
        insta::assert_snapshot!(
            env.render_ok(r#""description 123".contains(description.first_line())"#),
            @"true");

        // String patterns are not stringifiable
        insta::assert_snapshot!(env.parse_err(r#""fa".starts_with(regex:'[a-f]o+')"#), @r#"
         --> 1:18
          |
        1 | "fa".starts_with(regex:'[a-f]o+')
          |                  ^-------------^
          |
          = String patterns may not be used as expression values
        "#);

        // inner template error should propagate
        insta::assert_snapshot!(env.render_ok(r#""foo".contains(bad_string)"#), @"<Error: Bad>");
        insta::assert_snapshot!(
            env.render_ok(r#""foo".contains("f" ++ bad_string) ++ "bar""#), @"<Error: Bad>bar");
        insta::assert_snapshot!(
            env.render_ok(r#""foo".contains(separate("o", "f", bad_string))"#), @"<Error: Bad>");

        insta::assert_snapshot!(env.render_ok(r#""fooo".match(regex:'[a-f]o+')"#), @"fooo");
        insta::assert_snapshot!(env.render_ok(r#""fa".match(regex:'[a-f]o+')"#), @"");
        insta::assert_snapshot!(env.render_ok(r#""hello".match(regex:"h(ell)o")"#), @"hello");
        insta::assert_snapshot!(env.render_ok(r#""HEllo".match(regex-i:"h(ell)o")"#), @"HEllo");
        insta::assert_snapshot!(env.render_ok(r#""hEllo".match(glob:"h*o")"#), @"hEllo");
        insta::assert_snapshot!(env.render_ok(r#""Hello".match(glob:"h*o")"#), @"");
        insta::assert_snapshot!(env.render_ok(r#""HEllo".match(glob-i:"h*o")"#), @"HEllo");
        insta::assert_snapshot!(env.render_ok(r#""hello".match("he")"#), @"he");
        insta::assert_snapshot!(env.render_ok(r#""hello".match(substring:"he")"#), @"he");
        insta::assert_snapshot!(env.render_ok(r#""hello".match(exact:"he")"#), @"");

        // Evil regexes can cause invalid UTF-8 output, which nothing can
        // really be done about given we're matching against non-UTF-8 stuff a
        // lot as well.
        insta::assert_snapshot!(env.render_ok(r#""🥺".match(regex:'(?-u)^(?:.)')"#), @"<Error: incomplete utf-8 byte sequence from index 0>");

        insta::assert_snapshot!(env.parse_err(r#""🥺".match(not-a-pattern:"abc")"#), @r#"
         --> 1:11
          |
        1 | "🥺".match(not-a-pattern:"abc")
          |           ^-----------------^
          |
          = Bad string pattern
        Invalid string pattern kind `not-a-pattern:`
        "#);

        insta::assert_snapshot!(env.render_ok(r#""".first_line()"#), @"");
        insta::assert_snapshot!(env.render_ok(r#""foo\nbar".first_line()"#), @"foo");

        insta::assert_snapshot!(env.render_ok(r#""".lines()"#), @"");
        insta::assert_snapshot!(env.render_ok(r#""a\nb\nc\n".lines()"#), @"a b c");

        insta::assert_snapshot!(env.render_ok(r#""".split(",")"#), @"");
        insta::assert_snapshot!(env.render_ok(r#""a,b,c".split(",")"#), @"a b c");
        insta::assert_snapshot!(env.render_ok(r#""a::b::c::d".split("::")"#), @"a b c d");
        insta::assert_snapshot!(env.render_ok(r#""a,b,c,d".split(",", 0)"#), @"");
        insta::assert_snapshot!(env.render_ok(r#""a,b,c,d".split(",", 2)"#), @"a b,c,d");
        insta::assert_snapshot!(env.render_ok(r#""a,b,c,d".split(",", 3)"#), @"a b c,d");
        insta::assert_snapshot!(env.render_ok(r#""a,b,c,d".split(",", 10)"#), @"a b c d");
        insta::assert_snapshot!(env.render_ok(r#""abc".split(",", -1)"#), @"<Error: out of range integral type conversion attempted>");
        insta::assert_snapshot!(env.render_ok(r#"json("a1b2c3".split(regex:'\d+'))"#), @r#"["a","b","c",""]"#);
        insta::assert_snapshot!(env.render_ok(r#""foo  bar   baz".split(regex:'\s+')"#), @"foo bar baz");
        insta::assert_snapshot!(env.render_ok(r#""a1b2c3d4".split(regex:'\d+', 3)"#), @"a b c3d4");
        insta::assert_snapshot!(env.render_ok(r#"json("hello world".split(regex-i:"WORLD"))"#), @r#"["hello ",""]"#);

        insta::assert_snapshot!(env.render_ok(r#""".starts_with("")"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#""everything".starts_with("")"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#""".starts_with("foo")"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#""foo".starts_with("foo")"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#""foobar".starts_with("foo")"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#""foobar".starts_with("bar")"#), @"false");

        insta::assert_snapshot!(env.render_ok(r#""".ends_with("")"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#""everything".ends_with("")"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#""".ends_with("foo")"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#""foo".ends_with("foo")"#), @"true");
        insta::assert_snapshot!(env.render_ok(r#""foobar".ends_with("foo")"#), @"false");
        insta::assert_snapshot!(env.render_ok(r#""foobar".ends_with("bar")"#), @"true");

        insta::assert_snapshot!(env.render_ok(r#""".remove_prefix("wip: ")"#), @"");
        insta::assert_snapshot!(
            env.render_ok(r#""wip: testing".remove_prefix("wip: ")"#),
            @"testing");

        insta::assert_snapshot!(
            env.render_ok(r#""bar@my.example.com".remove_suffix("@other.example.com")"#),
            @"bar@my.example.com");
        insta::assert_snapshot!(
            env.render_ok(r#""bar@other.example.com".remove_suffix("@other.example.com")"#),
            @"bar");

        insta::assert_snapshot!(env.render_ok(r#"" \n \r    \t \r ".trim()"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"" \n \r foo  bar \t \r ".trim()"#), @"foo  bar");

        insta::assert_snapshot!(env.render_ok(r#"" \n \r    \t \r ".trim_start()"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"" \n \r foo  bar \t \r ".trim_start()"#), @"foo  bar");

        insta::assert_snapshot!(env.render_ok(r#"" \n \r    \t \r ".trim_end()"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"" \n \r foo  bar \t \r ".trim_end()"#), @" foo  bar");

        insta::assert_snapshot!(env.render_ok(r#""foo".substr(0, 0)"#), @"");
        insta::assert_snapshot!(env.render_ok(r#""foo".substr(0, 1)"#), @"f");
        insta::assert_snapshot!(env.render_ok(r#""foo".substr(0, 3)"#), @"foo");
        insta::assert_snapshot!(env.render_ok(r#""foo".substr(0, 4)"#), @"foo");
        insta::assert_snapshot!(env.render_ok(r#""abcdef".substr(2, -1)"#), @"cde");
        insta::assert_snapshot!(env.render_ok(r#""abcdef".substr(-3, 99)"#), @"def");
        insta::assert_snapshot!(env.render_ok(r#""abcdef".substr(-6, 99)"#), @"abcdef");
        insta::assert_snapshot!(env.render_ok(r#""abcdef".substr(-7, 1)"#), @"a");

        // non-ascii characters
        insta::assert_snapshot!(env.render_ok(r#""abc💩".substr(2, -1)"#), @"c💩");
        insta::assert_snapshot!(env.render_ok(r#""abc💩".substr(3, -3)"#), @"💩");
        insta::assert_snapshot!(env.render_ok(r#""abc💩".substr(3, -4)"#), @"");
        insta::assert_snapshot!(env.render_ok(r#""abc💩".substr(6, -3)"#), @"💩");
        insta::assert_snapshot!(env.render_ok(r#""abc💩".substr(7, -3)"#), @"");
        insta::assert_snapshot!(env.render_ok(r#""abc💩".substr(3, 4)"#), @"");
        insta::assert_snapshot!(env.render_ok(r#""abc💩".substr(3, 6)"#), @"");
        insta::assert_snapshot!(env.render_ok(r#""abc💩".substr(3, 7)"#), @"💩");
        insta::assert_snapshot!(env.render_ok(r#""abc💩".substr(-1, 7)"#), @"");
        insta::assert_snapshot!(env.render_ok(r#""abc💩".substr(-3, 7)"#), @"");
        insta::assert_snapshot!(env.render_ok(r#""abc💩".substr(-4, 7)"#), @"💩");

        // ranges with end > start are empty
        insta::assert_snapshot!(env.render_ok(r#""abcdef".substr(4, 2)"#), @"");
        insta::assert_snapshot!(env.render_ok(r#""abcdef".substr(-2, -4)"#), @"");

        insta::assert_snapshot!(env.render_ok(r#""hello".escape_json()"#), @r#""hello""#);
        insta::assert_snapshot!(env.render_ok(r#""he \n ll \n \" o".escape_json()"#), @r#""he \n ll \n \" o""#);

        // simple substring replacement
        insta::assert_snapshot!(env.render_ok(r#""hello world".replace("world", "jj")"#), @"hello jj");
        insta::assert_snapshot!(env.render_ok(r#""hello world world".replace("world", "jj")"#), @"hello jj jj");
        insta::assert_snapshot!(env.render_ok(r#""hello".replace("missing", "jj")"#), @"hello");

        // replace with limit >=0
        insta::assert_snapshot!(env.render_ok(r#""hello world world".replace("world", "jj", 0)"#), @"hello world world");
        insta::assert_snapshot!(env.render_ok(r#""hello world world".replace("world", "jj", 1)"#), @"hello jj world");
        insta::assert_snapshot!(env.render_ok(r#""hello world world world".replace("world", "jj", 2)"#), @"hello jj jj world");

        // replace with limit <0 (error due to negative limit)
        insta::assert_snapshot!(env.render_ok(r#""hello world world".replace("world", "jj", -1)"#), @"<Error: out of range integral type conversion attempted>");
        insta::assert_snapshot!(env.render_ok(r#""hello world world".replace("world", "jj", -5)"#), @"<Error: out of range integral type conversion attempted>");

        // replace with regex patterns
        insta::assert_snapshot!(env.render_ok(r#""hello123world456".replace(regex:'\d+', "X")"#), @"helloXworldX");
        insta::assert_snapshot!(env.render_ok(r#""hello123world456".replace(regex:'\d+', "X", 1)"#), @"helloXworld456");

        // replace with regex patterns (capture groups)
        insta::assert_snapshot!(env.render_ok(r#""HELLO    WORLD".replace(regex-i:"(hello) +(world)", "$2 $1")"#), @"WORLD HELLO");
        insta::assert_snapshot!(env.render_ok(r#""abc123".replace(regex:"([a-z]+)([0-9]+)", "$2-$1")"#), @"123-abc");
        insta::assert_snapshot!(env.render_ok(r#""foo123bar".replace(regex:'\d+', "[$0]")"#), @"foo[123]bar");

        // replace with regex patterns (case insensitive)
        insta::assert_snapshot!(env.render_ok(r#""Hello World".replace(regex-i:"hello", "hi")"#), @"hi World");
        insta::assert_snapshot!(env.render_ok(r#""Hello World Hello".replace(regex-i:"hello", "hi")"#), @"hi World hi");
        insta::assert_snapshot!(env.render_ok(r#""Hello World Hello".replace(regex-i:"hello", "hi", 1)"#), @"hi World Hello");

        // replace with strings that look regex-y ($n patterns are always expanded)
        insta::assert_snapshot!(env.render_ok(r#"'hello\d+world'.replace('\d+', "X")"#), @"helloXworld");
        insta::assert_snapshot!(env.render_ok(r#""(foo)($1)bar".replace("$1", "$2")"#), @"(foo)()bar");
        insta::assert_snapshot!(env.render_ok(r#""test(abc)end".replace("(abc)", "X")"#), @"testXend");

        // replace with templates
        insta::assert_snapshot!(env.render_ok(r#""hello world".replace("world", description.first_line())"#), @"hello description 1");

        // replace with error
        insta::assert_snapshot!(env.render_ok(r#""hello world".replace("world", bad_string)"#), @"<Error: Bad>");
    }

    #[test]
    fn test_config_value_method() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("boolean", || literal(ConfigValue::from(true)));
        env.add_keyword("integer", || literal(ConfigValue::from(42)));
        env.add_keyword("string", || literal(ConfigValue::from("foo")));
        env.add_keyword("string_list", || {
            literal(ConfigValue::from_iter(["foo", "bar"]))
        });

        insta::assert_snapshot!(env.render_ok("boolean"), @"true");
        insta::assert_snapshot!(env.render_ok("integer"), @"42");
        insta::assert_snapshot!(env.render_ok("string"), @r#""foo""#);
        insta::assert_snapshot!(env.render_ok("string_list"), @r#"["foo", "bar"]"#);

        insta::assert_snapshot!(env.render_ok("boolean.as_boolean()"), @"true");
        insta::assert_snapshot!(env.render_ok("integer.as_integer()"), @"42");
        insta::assert_snapshot!(env.render_ok("string.as_string()"), @"foo");
        insta::assert_snapshot!(env.render_ok("string_list.as_string_list()"), @"foo bar");

        insta::assert_snapshot!(
            env.render_ok("boolean.as_integer()"),
            @"<Error: invalid type: boolean `true`, expected i64>");
        insta::assert_snapshot!(
            env.render_ok("integer.as_string()"),
            @"<Error: invalid type: integer `42`, expected a string>");
        insta::assert_snapshot!(
            env.render_ok("string.as_string_list()"),
            @r#"<Error: invalid type: string "foo", expected a sequence>"#);
        insta::assert_snapshot!(
            env.render_ok("string_list.as_boolean()"),
            @"<Error: invalid type: sequence, expected a boolean>");
    }

    #[test]
    fn test_signature() {
        let mut env = TestTemplateEnv::new();

        env.add_keyword("author", || {
            literal(new_signature("Test User", "test.user@example.com"))
        });
        insta::assert_snapshot!(env.render_ok(r#"author"#), @"Test User <test.user@example.com>");
        insta::assert_snapshot!(env.render_ok(r#"author.name()"#), @"Test User");
        insta::assert_snapshot!(env.render_ok(r#"author.email()"#), @"test.user@example.com");

        env.add_keyword("author", || {
            literal(new_signature("Another Test User", "test.user@example.com"))
        });
        insta::assert_snapshot!(env.render_ok(r#"author"#), @"Another Test User <test.user@example.com>");
        insta::assert_snapshot!(env.render_ok(r#"author.name()"#), @"Another Test User");
        insta::assert_snapshot!(env.render_ok(r#"author.email()"#), @"test.user@example.com");

        env.add_keyword("author", || {
            literal(new_signature("Test User", "test.user@invalid@example.com"))
        });
        insta::assert_snapshot!(env.render_ok(r#"author"#), @"Test User <test.user@invalid@example.com>");
        insta::assert_snapshot!(env.render_ok(r#"author.name()"#), @"Test User");
        insta::assert_snapshot!(env.render_ok(r#"author.email()"#), @"test.user@invalid@example.com");

        env.add_keyword("author", || {
            literal(new_signature("Test User", "test.user"))
        });
        insta::assert_snapshot!(env.render_ok(r#"author"#), @"Test User <test.user>");
        insta::assert_snapshot!(env.render_ok(r#"author.email()"#), @"test.user");

        env.add_keyword("author", || {
            literal(new_signature("Test User", "test.user+tag@example.com"))
        });
        insta::assert_snapshot!(env.render_ok(r#"author"#), @"Test User <test.user+tag@example.com>");
        insta::assert_snapshot!(env.render_ok(r#"author.email()"#), @"test.user+tag@example.com");

        env.add_keyword("author", || literal(new_signature("Test User", "x@y")));
        insta::assert_snapshot!(env.render_ok(r#"author"#), @"Test User <x@y>");
        insta::assert_snapshot!(env.render_ok(r#"author.email()"#), @"x@y");

        env.add_keyword("author", || {
            literal(new_signature("", "test.user@example.com"))
        });
        insta::assert_snapshot!(env.render_ok(r#"author"#), @"<test.user@example.com>");
        insta::assert_snapshot!(env.render_ok(r#"author.name()"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"author.email()"#), @"test.user@example.com");

        env.add_keyword("author", || literal(new_signature("Test User", "")));
        insta::assert_snapshot!(env.render_ok(r#"author"#), @"Test User");
        insta::assert_snapshot!(env.render_ok(r#"author.name()"#), @"Test User");
        insta::assert_snapshot!(env.render_ok(r#"author.email()"#), @"");

        env.add_keyword("author", || literal(new_signature("", "")));
        insta::assert_snapshot!(env.render_ok(r#"author"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"author.name()"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"author.email()"#), @"");
    }

    #[test]
    fn test_size_hint_method() {
        let mut env = TestTemplateEnv::new();

        env.add_keyword("unbounded", || literal((5, None)));
        insta::assert_snapshot!(env.render_ok(r#"unbounded.lower()"#), @"5");
        insta::assert_snapshot!(env.render_ok(r#"unbounded.upper()"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"unbounded.exact()"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"unbounded.zero()"#), @"false");

        env.add_keyword("bounded", || literal((0, Some(10))));
        insta::assert_snapshot!(env.render_ok(r#"bounded.lower()"#), @"0");
        insta::assert_snapshot!(env.render_ok(r#"bounded.upper()"#), @"10");
        insta::assert_snapshot!(env.render_ok(r#"bounded.exact()"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"bounded.zero()"#), @"false");

        env.add_keyword("zero", || literal((0, Some(0))));
        insta::assert_snapshot!(env.render_ok(r#"zero.lower()"#), @"0");
        insta::assert_snapshot!(env.render_ok(r#"zero.upper()"#), @"0");
        insta::assert_snapshot!(env.render_ok(r#"zero.exact()"#), @"0");
        insta::assert_snapshot!(env.render_ok(r#"zero.zero()"#), @"true");
    }

    #[test]
    fn test_timestamp_method() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("t0", || literal(new_timestamp(0, 0)));

        insta::assert_snapshot!(
            env.render_ok(r#"t0.format("%Y%m%d %H:%M:%S")"#),
            @"19700101 00:00:00");

        // Invalid format string
        insta::assert_snapshot!(env.parse_err(r#"t0.format("%_")"#), @r#"
         --> 1:11
          |
        1 | t0.format("%_")
          |           ^--^
          |
          = Invalid time format
        "#);

        // Invalid type
        insta::assert_snapshot!(env.parse_err(r#"t0.format(0)"#), @r"
         --> 1:11
          |
        1 | t0.format(0)
          |           ^
          |
          = Expected string literal
        ");

        // Dynamic string isn't supported yet
        insta::assert_snapshot!(env.parse_err(r#"t0.format("%Y" ++ "%m")"#), @r#"
         --> 1:11
          |
        1 | t0.format("%Y" ++ "%m")
          |           ^----------^
          |
          = Expected string literal
        "#);

        // Literal alias expansion
        env.add_alias("time_format", r#""%Y-%m-%d""#);
        env.add_alias("bad_time_format", r#""%_""#);
        insta::assert_snapshot!(env.render_ok(r#"t0.format(time_format)"#), @"1970-01-01");
        insta::assert_snapshot!(env.parse_err(r#"t0.format(bad_time_format)"#), @r#"
         --> 1:11
          |
        1 | t0.format(bad_time_format)
          |           ^-------------^
          |
          = In alias `bad_time_format`
         --> 1:1
          |
        1 | "%_"
          | ^--^
          |
          = Invalid time format
        "#);
    }

    #[test]
    fn test_fill_function() {
        let mut env = TestTemplateEnv::new();
        env.add_color("error", crossterm::style::Color::DarkRed);

        insta::assert_snapshot!(
            env.render_ok(r#"fill(20, "The quick fox jumps over the " ++
                                  label("error", "lazy") ++ " dog\n")"#),
            @r"
        The quick fox jumps
        over the [38;5;1mlazy[39m dog
        ");

        // A low value will not chop words, but can chop a label by words
        insta::assert_snapshot!(
            env.render_ok(r#"fill(9, "Longlonglongword an some short words " ++
                                  label("error", "longlonglongword and short words") ++
                                  " back out\n")"#),
            @r"
        Longlonglongword
        an some
        short
        words
        [38;5;1mlonglonglongword[39m
        [38;5;1mand short[39m
        [38;5;1mwords[39m
        back out
        ");

        // Filling to 0 means breaking at every word
        insta::assert_snapshot!(
            env.render_ok(r#"fill(0, "The quick fox jumps over the " ++
                                  label("error", "lazy") ++ " dog\n")"#),
            @r"
        The
        quick
        fox
        jumps
        over
        the
        [38;5;1mlazy[39m
        dog
        ");

        // Filling to -0 is the same as 0
        insta::assert_snapshot!(
            env.render_ok(r#"fill(-0, "The quick fox jumps over the " ++
                                  label("error", "lazy") ++ " dog\n")"#),
            @r"
        The
        quick
        fox
        jumps
        over
        the
        [38;5;1mlazy[39m
        dog
        ");

        // Filling to negative width is an error
        insta::assert_snapshot!(
            env.render_ok(r#"fill(-10, "The quick fox jumps over the " ++
                                  label("error", "lazy") ++ " dog\n")"#),
            @"[38;5;1m<Error: out of range integral type conversion attempted>[39m");

        // Word-wrap, then indent
        insta::assert_snapshot!(
            env.render_ok(r#""START marker to help insta\n" ++
                             indent("    ", fill(20, "The quick fox jumps over the " ++
                                                 label("error", "lazy") ++ " dog\n"))"#),
            @r"
        START marker to help insta
            The quick fox jumps
            over the [38;5;1mlazy[39m dog
        ");

        // Word-wrap indented (no special handling for leading spaces)
        insta::assert_snapshot!(
            env.render_ok(r#""START marker to help insta\n" ++
                             fill(20, indent("    ", "The quick fox jumps over the " ++
                                             label("error", "lazy") ++ " dog\n"))"#),
            @r"
        START marker to help insta
            The quick fox
        jumps over the [38;5;1mlazy[39m
        dog
        ");
    }

    #[test]
    fn test_indent_function() {
        let mut env = TestTemplateEnv::new();
        env.add_color("error", crossterm::style::Color::DarkRed);
        env.add_color("warning", crossterm::style::Color::DarkYellow);
        env.add_color("hint", crossterm::style::Color::DarkCyan);

        // Empty line shouldn't be indented. Not using insta here because we test
        // whitespace existence.
        assert_eq!(env.render_ok(r#"indent("__", "")"#), "");
        assert_eq!(env.render_ok(r#"indent("__", "\n")"#), "\n");
        assert_eq!(env.render_ok(r#"indent("__", "a\n\nb")"#), "__a\n\n__b");

        // "\n" at end of labeled text
        insta::assert_snapshot!(
            env.render_ok(r#"indent("__", label("error", "a\n") ++ label("warning", "b\n"))"#),
            @r"
        [38;5;1m__a[39m
        [38;5;3m__b[39m
        ");

        // "\n" in labeled text
        insta::assert_snapshot!(
            env.render_ok(r#"indent("__", label("error", "a") ++ label("warning", "b\nc"))"#),
            @r"
        [38;5;1m__a[38;5;3mb[39m
        [38;5;3m__c[39m
        ");

        // Labeled prefix + unlabeled content
        insta::assert_snapshot!(
            env.render_ok(r#"indent(label("error", "XX"), "a\nb\n")"#),
            @r"
        [38;5;1mXX[39ma
        [38;5;1mXX[39mb
        ");

        // Nested indent, silly but works
        insta::assert_snapshot!(
            env.render_ok(r#"indent(label("hint", "A"),
                                    label("warning", indent(label("hint", "B"),
                                                            label("error", "x\n") ++ "y")))"#),
            @r"
        [38;5;6mAB[38;5;1mx[39m
        [38;5;6mAB[38;5;3my[39m
        ");
    }

    #[test]
    fn test_pad_function() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("bad_string", || new_error_property::<String>("Bad"));
        env.add_color("red", crossterm::style::Color::Red);
        env.add_color("cyan", crossterm::style::Color::DarkCyan);

        // Default fill_char is ' '
        insta::assert_snapshot!(
            env.render_ok(r"'{' ++ pad_start(5, label('red', 'foo')) ++ '}'"),
            @"{  [38;5;9mfoo[39m}");
        insta::assert_snapshot!(
            env.render_ok(r"'{' ++ pad_end(5, label('red', 'foo')) ++ '}'"),
            @"{[38;5;9mfoo[39m  }");
        insta::assert_snapshot!(
            env.render_ok(r"'{' ++ pad_centered(5, label('red', 'foo')) ++ '}'"),
            @"{ [38;5;9mfoo[39m }");

        // Labeled fill char
        insta::assert_snapshot!(
            env.render_ok(r"pad_start(5, label('red', 'foo'), fill_char=label('cyan', '='))"),
            @"[38;5;6m==[38;5;9mfoo[39m");
        insta::assert_snapshot!(
            env.render_ok(r"pad_end(5, label('red', 'foo'), fill_char=label('cyan', '='))"),
            @"[38;5;9mfoo[38;5;6m==[39m");
        insta::assert_snapshot!(
            env.render_ok(r"pad_centered(5, label('red', 'foo'), fill_char=label('cyan', '='))"),
            @"[38;5;6m=[38;5;9mfoo[38;5;6m=[39m");

        // Error in fill char: the output looks odd (because the error message
        // isn't 1-width character), but is still readable.
        insta::assert_snapshot!(
            env.render_ok(r"pad_start(3, 'foo', fill_char=bad_string)"),
            @"foo");
        insta::assert_snapshot!(
            env.render_ok(r"pad_end(5, 'foo', fill_char=bad_string)"),
            @"foo<<Error: Error: Bad>Bad>");
        insta::assert_snapshot!(
            env.render_ok(r"pad_centered(5, 'foo', fill_char=bad_string)"),
            @"<Error: Bad>foo<Error: Bad>");
    }

    #[test]
    fn test_truncate_function() {
        let mut env = TestTemplateEnv::new();
        env.add_color("red", crossterm::style::Color::Red);

        insta::assert_snapshot!(
            env.render_ok(r"truncate_start(2, label('red', 'foobar')) ++ 'baz'"),
            @"[38;5;9mar[39mbaz");
        insta::assert_snapshot!(
            env.render_ok(r"truncate_end(2, label('red', 'foobar')) ++ 'baz'"),
            @"[38;5;9mfo[39mbaz");
    }

    #[test]
    fn test_label_function() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("empty", || literal(true));
        env.add_color("error", crossterm::style::Color::DarkRed);
        env.add_color("warning", crossterm::style::Color::DarkYellow);

        // Literal
        insta::assert_snapshot!(
            env.render_ok(r#"label("error", "text")"#),
            @"[38;5;1mtext[39m");

        // Evaluated property
        insta::assert_snapshot!(
            env.render_ok(r#"label("error".first_line(), "text")"#),
            @"[38;5;1mtext[39m");

        // Template
        insta::assert_snapshot!(
            env.render_ok(r#"label(if(empty, "error", "warning"), "text")"#),
            @"[38;5;1mtext[39m");
    }

    #[test]
    fn test_raw_escape_sequence_function_strip_labels() {
        let mut env = TestTemplateEnv::new();
        env.add_color("error", crossterm::style::Color::DarkRed);
        env.add_color("warning", crossterm::style::Color::DarkYellow);

        insta::assert_snapshot!(
            env.render_ok(r#"raw_escape_sequence(label("error warning", "text"))"#),
            @"text",
        );
    }

    #[test]
    fn test_raw_escape_sequence_function_ansi_escape() {
        let env = TestTemplateEnv::new();

        // Sanitize ANSI escape without raw_escape_sequence
        insta::assert_snapshot!(env.render_ok(r#""\e""#), @"␛");
        insta::assert_snapshot!(env.render_ok(r#""\x1b""#), @"␛");
        insta::assert_snapshot!(env.render_ok(r#""\x1B""#), @"␛");
        insta::assert_snapshot!(
            env.render_ok(r#""]8;;"
                ++ "http://example.com"
                ++ "\e\\"
                ++ "Example"
                ++ "\x1b]8;;\x1B\\""#),
            @r"␛]8;;http://example.com␛\Example␛]8;;␛\");

        // Don't sanitize ANSI escape with raw_escape_sequence
        insta::assert_snapshot!(env.render_ok(r#"raw_escape_sequence("\e")"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"raw_escape_sequence("\x1b")"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"raw_escape_sequence("\x1B")"#), @"");
        insta::assert_snapshot!(
            env.render_ok(r#"raw_escape_sequence("]8;;"
                ++ "http://example.com"
                ++ "\e\\"
                ++ "Example"
                ++ "\x1b]8;;\x1B\\")"#),
            @r"]8;;http://example.com\Example]8;;\");
    }

    #[test]
    fn test_stringify_function() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("none_i64", || literal(None::<i64>));
        env.add_color("error", crossterm::style::Color::DarkRed);

        insta::assert_snapshot!(env.render_ok("stringify(false)"), @"false");
        insta::assert_snapshot!(env.render_ok("stringify(42).len()"), @"2");
        insta::assert_snapshot!(env.render_ok("stringify(none_i64)"), @"");
        insta::assert_snapshot!(env.render_ok("stringify(label('error', 'text'))"), @"text");
    }

    #[test]
    fn test_json_function() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("none_i64", || literal(None::<i64>));
        env.add_keyword("string_list", || {
            literal(vec!["foo".to_owned(), "bar".to_owned()])
        });
        env.add_keyword("config_value_table", || {
            literal(ConfigValue::from_iter([("foo", "bar")]))
        });
        env.add_keyword("signature", || {
            literal(Signature {
                name: "Test User".to_owned(),
                email: "test.user@example.com".to_owned(),
                timestamp: Timestamp {
                    timestamp: MillisSinceEpoch(0),
                    tz_offset: 0,
                },
            })
        });
        env.add_keyword("email", || literal(Email("foo@bar".to_owned())));
        env.add_keyword("size_hint", || literal((5, None)));
        env.add_keyword("timestamp", || {
            literal(Timestamp {
                timestamp: MillisSinceEpoch(0),
                tz_offset: 0,
            })
        });
        env.add_keyword("timestamp_range", || {
            literal(TimestampRange {
                start: Timestamp {
                    timestamp: MillisSinceEpoch(0),
                    tz_offset: 0,
                },
                end: Timestamp {
                    timestamp: MillisSinceEpoch(86_400_000),
                    tz_offset: -60,
                },
            })
        });

        insta::assert_snapshot!(env.render_ok(r#"json('"quoted"')"#), @r#""\"quoted\"""#);
        insta::assert_snapshot!(env.render_ok(r#"json(string_list)"#), @r#"["foo","bar"]"#);
        insta::assert_snapshot!(env.render_ok("json(false)"), @"false");
        insta::assert_snapshot!(env.render_ok("json(42)"), @"42");
        insta::assert_snapshot!(env.render_ok("json(none_i64)"), @"null");
        insta::assert_snapshot!(env.render_ok(r#"json(config_value_table)"#), @r#"{"foo":"bar"}"#);
        insta::assert_snapshot!(env.render_ok("json(email)"), @r#""foo@bar""#);
        insta::assert_snapshot!(
            env.render_ok("json(signature)"),
            @r#"{"name":"Test User","email":"test.user@example.com","timestamp":"1970-01-01T00:00:00Z"}"#);
        insta::assert_snapshot!(env.render_ok("json(size_hint)"), @"[5,null]");
        insta::assert_snapshot!(env.render_ok("json(timestamp)"), @r#""1970-01-01T00:00:00Z""#);
        insta::assert_snapshot!(
            env.render_ok("json(timestamp_range)"),
            @r#"{"start":"1970-01-01T00:00:00Z","end":"1970-01-01T23:00:00-01:00"}"#);

        insta::assert_snapshot!(env.parse_err(r#"json(string_list.map(|s| s))"#), @r"
         --> 1:6
          |
        1 | json(string_list.map(|s| s))
          |      ^--------------------^
          |
          = Expected expression of type `Serialize`, but actual type is `ListTemplate`
        ");
    }

    #[test]
    fn test_coalesce_function() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("bad_string", || new_error_property::<String>("Bad"));
        env.add_keyword("empty_string", || literal("".to_owned()));
        env.add_keyword("non_empty_string", || literal("a".to_owned()));

        insta::assert_snapshot!(env.render_ok(r#"coalesce()"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"coalesce("")"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"coalesce("", "a", "", "b")"#), @"a");
        insta::assert_snapshot!(
            env.render_ok(r#"coalesce(empty_string, "", non_empty_string)"#), @"a");

        // "false" is not empty
        insta::assert_snapshot!(env.render_ok(r#"coalesce(false, true)"#), @"false");

        // Error is not empty
        insta::assert_snapshot!(env.render_ok(r#"coalesce(bad_string, "a")"#), @"<Error: Bad>");
        // but can be short-circuited
        insta::assert_snapshot!(env.render_ok(r#"coalesce("a", bad_string)"#), @"a");

        // Keyword arguments are rejected.
        insta::assert_snapshot!(env.parse_err(r#"coalesce("a", value2="b")"#), @r#"
         --> 1:15
          |
        1 | coalesce("a", value2="b")
          |               ^--------^
          |
          = Function `coalesce`: Unexpected keyword arguments
        "#);
    }

    #[test]
    fn test_concat_function() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("empty", || literal(true));
        env.add_keyword("hidden", || literal(false));
        env.add_color("empty", crossterm::style::Color::DarkGreen);
        env.add_color("error", crossterm::style::Color::DarkRed);
        env.add_color("warning", crossterm::style::Color::DarkYellow);

        insta::assert_snapshot!(env.render_ok(r#"concat()"#), @"");
        insta::assert_snapshot!(
            env.render_ok(r#"concat(hidden, empty)"#),
            @"false[38;5;2mtrue[39m");
        insta::assert_snapshot!(
            env.render_ok(r#"concat(label("error", ""), label("warning", "a"), "b")"#),
            @"[38;5;3ma[39mb");

        // Keyword arguments are rejected.
        insta::assert_snapshot!(env.parse_err(r#"concat("a", value2="b")"#), @r#"
         --> 1:13
          |
        1 | concat("a", value2="b")
          |             ^--------^
          |
          = Function `concat`: Unexpected keyword arguments
        "#);
    }

    #[test]
    fn test_join_function() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("description", || literal("".to_owned()));
        env.add_keyword("empty", || literal(true));
        env.add_keyword("hidden", || literal(false));
        env.add_color("empty", crossterm::style::Color::DarkGreen);
        env.add_color("error", crossterm::style::Color::DarkRed);
        env.add_color("warning", crossterm::style::Color::DarkYellow);

        // Template literals.
        insta::assert_snapshot!(env.render_ok(r#"join(",")"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"join(",", "")"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"join(",", "a")"#), @"a");
        insta::assert_snapshot!(env.render_ok(r#"join(",", "a", "b")"#), @"a,b");
        insta::assert_snapshot!(env.render_ok(r#"join(",", "a", "", "b")"#), @"a,,b");
        insta::assert_snapshot!(env.render_ok(r#"join(",", "a", "b", "")"#), @"a,b,");
        insta::assert_snapshot!(env.render_ok(r#"join(",", "", "a", "b")"#), @",a,b");
        insta::assert_snapshot!(
            env.render_ok(r#"join("--", 1, "", true, "test", "")"#),
            @"1----true--test--");

        // Separator is required.
        insta::assert_snapshot!(env.parse_err(r#"join()"#), @r"
         --> 1:6
          |
        1 | join()
          |      ^
          |
          = Function `join`: Expected at least 1 arguments
        ");

        // Labeled.
        insta::assert_snapshot!(
            env.render_ok(r#"join(",", label("error", ""), label("warning", "a"), "b")"#),
            @",[38;5;3ma[39m,b");
        insta::assert_snapshot!(
            env.render_ok(
                r#"join(label("empty", "<>"), label("error", "a"), label("warning", ""), "b")"#),
            @"[38;5;1ma[38;5;2m<><>[39mb");

        // List template.
        insta::assert_snapshot!(env.render_ok(r#"join(",", "a", ("" ++ ""))"#), @"a,");
        insta::assert_snapshot!(env.render_ok(r#"join(",", "a", ("" ++ "b"))"#), @"a,b");

        // Nested.
        insta::assert_snapshot!(
            env.render_ok(r#"join(",", "a", join("|", "", ""))"#), @"a,|");
        insta::assert_snapshot!(
            env.render_ok(r#"join(",", "a", join("|", "b", ""))"#), @"a,b|");
        insta::assert_snapshot!(
            env.render_ok(r#"join(",", "a", join("|", "b", "c"))"#), @"a,b|c");

        // Keywords.
        insta::assert_snapshot!(
            env.render_ok(r#"join(",", hidden, description, empty)"#),
            @"false,,[38;5;2mtrue[39m");
        insta::assert_snapshot!(
            env.render_ok(r#"join(hidden, "X", "Y", "Z")"#),
            @"XfalseYfalseZ");
        insta::assert_snapshot!(
            env.render_ok(r#"join(hidden, empty)"#),
            @"[38;5;2mtrue[39m");

        // Keyword arguments are rejected.
        insta::assert_snapshot!(env.parse_err(r#"join(",", "a", arg="b")"#), @r#"
         --> 1:16
          |
        1 | join(",", "a", arg="b")
          |                ^-----^
          |
          = Function `join`: Unexpected keyword arguments
        "#);
    }

    #[test]
    fn test_separate_function() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("description", || literal("".to_owned()));
        env.add_keyword("empty", || literal(true));
        env.add_keyword("hidden", || literal(false));
        env.add_color("empty", crossterm::style::Color::DarkGreen);
        env.add_color("error", crossterm::style::Color::DarkRed);
        env.add_color("warning", crossterm::style::Color::DarkYellow);

        insta::assert_snapshot!(env.render_ok(r#"separate(" ")"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"separate(" ", "")"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"separate(" ", "a")"#), @"a");
        insta::assert_snapshot!(env.render_ok(r#"separate(" ", "a", "b")"#), @"a b");
        insta::assert_snapshot!(env.render_ok(r#"separate(" ", "a", "", "b")"#), @"a b");
        insta::assert_snapshot!(env.render_ok(r#"separate(" ", "a", "b", "")"#), @"a b");
        insta::assert_snapshot!(env.render_ok(r#"separate(" ", "", "a", "b")"#), @"a b");

        // Labeled
        insta::assert_snapshot!(
            env.render_ok(r#"separate(" ", label("error", ""), label("warning", "a"), "b")"#),
            @"[38;5;3ma[39m b");

        // List template
        insta::assert_snapshot!(env.render_ok(r#"separate(" ", "a", ("" ++ ""))"#), @"a");
        insta::assert_snapshot!(env.render_ok(r#"separate(" ", "a", ("" ++ "b"))"#), @"a b");

        // Nested separate
        insta::assert_snapshot!(
            env.render_ok(r#"separate(" ", "a", separate("|", "", ""))"#), @"a");
        insta::assert_snapshot!(
            env.render_ok(r#"separate(" ", "a", separate("|", "b", ""))"#), @"a b");
        insta::assert_snapshot!(
            env.render_ok(r#"separate(" ", "a", separate("|", "b", "c"))"#), @"a b|c");

        // Conditional template
        insta::assert_snapshot!(
            env.render_ok(r#"separate(" ", "a", if(true, ""))"#), @"a");
        insta::assert_snapshot!(
            env.render_ok(r#"separate(" ", "a", if(true, "", "f"))"#), @"a");
        insta::assert_snapshot!(
            env.render_ok(r#"separate(" ", "a", if(false, "t", ""))"#), @"a");
        insta::assert_snapshot!(
            env.render_ok(r#"separate(" ", "a", if(true, "t", "f"))"#), @"a t");

        // Separate keywords
        insta::assert_snapshot!(
            env.render_ok(r#"separate(" ", hidden, description, empty)"#),
            @"false [38;5;2mtrue[39m");

        // Keyword as separator
        insta::assert_snapshot!(
            env.render_ok(r#"separate(hidden, "X", "Y", "Z")"#),
            @"XfalseYfalseZ");

        // Keyword arguments are rejected.
        insta::assert_snapshot!(env.parse_err(r#"separate(" ", "a", value2="b")"#), @r#"
         --> 1:20
          |
        1 | separate(" ", "a", value2="b")
          |                    ^--------^
          |
          = Function `separate`: Unexpected keyword arguments
        "#);
    }

    #[test]
    fn test_surround_function() {
        let mut env = TestTemplateEnv::new();
        env.add_keyword("lt", || literal("<".to_owned()));
        env.add_keyword("gt", || literal(">".to_owned()));
        env.add_keyword("content", || literal("content".to_owned()));
        env.add_keyword("empty_content", || literal("".to_owned()));
        env.add_color("error", crossterm::style::Color::DarkRed);
        env.add_color("paren", crossterm::style::Color::Cyan);

        insta::assert_snapshot!(env.render_ok(r#"surround("{", "}", "")"#), @"");
        insta::assert_snapshot!(env.render_ok(r#"surround("{", "}", "a")"#), @"{a}");

        // Labeled
        insta::assert_snapshot!(
            env.render_ok(
                r#"surround(label("paren", "("), label("paren", ")"), label("error", "a"))"#),
            @"[38;5;14m([38;5;1ma[38;5;14m)[39m");

        // Keyword
        insta::assert_snapshot!(
            env.render_ok(r#"surround(lt, gt, content)"#),
            @"<content>");
        insta::assert_snapshot!(
            env.render_ok(r#"surround(lt, gt, empty_content)"#),
            @"");

        // Conditional template as content
        insta::assert_snapshot!(
            env.render_ok(r#"surround(lt, gt, if(empty_content, "", "empty"))"#),
            @"<empty>");
        insta::assert_snapshot!(
            env.render_ok(r#"surround(lt, gt, if(empty_content, "not empty", ""))"#),
            @"");
    }
}
