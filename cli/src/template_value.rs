// Copyright 2026 The Jujutsu Authors
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

//! Value types, conversion, and registration helpers for templates.

use std::any::Any;
use std::any::TypeId;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;

use bstr::BString;

use crate::templater::Template;

/// Marker for opaque template value types.
pub trait OpaqueTemplateValue: Clone + 'static {}

/// Type that can be converted to/from the template runtime representation.
#[diagnostic::on_unimplemented(
    message = "`OpaqueTemplateValue` not implemented for the template value type `{Self}`"
)]
pub trait TemplateValue: Sized + 'static {
    /// Returns the descriptor for this type.
    fn value_type() -> TemplateValueType;

    /// Extracts a typed value.
    fn try_from_repr(repr: TemplateValueRepr) -> Result<Self, TemplateValueExtractionError>;

    /// Converts this value into its type-erased representation.
    fn into_repr(self) -> TemplateValueRepr;
}

/// Error extracting a typed value from its representation.
#[derive(Debug)]
pub enum TemplateValueExtractionError {
    UnexpectedType(TemplateValueType),
    MissingValue,
}

/// Describes template value types.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum TemplateValueType {
    Scalar(TypeId),
    Option(TypeId),
}

/// Type-erased template runtime value.
///
/// For types supporting `Option<T>`, `T` and `Some(T)` share a representation
/// and can be extracted as either `T` or `Option<T>`. This allows optional
/// values to be implicitly unwrapped as `T`, including in comparisons.
///
/// The declared type still matters. `Option<T>` can be coerced to a boolean,
/// whereas `T` cannot.
#[derive(Clone, Debug)]
pub struct TemplateValueRepr {
    tagged: Tagged,
}

#[derive(Clone, Debug)]
enum Tagged {
    // Inline common value types:
    // ByteString(BString),
    // String(String),
    // Boolean(bool),
    // Integer(Option<i64>),
    // Opaque value, or its type if no value is available:
    OpaqueSome(Rc<dyn Any>),
    OpaqueNone(TypeId),
    // We'll probably add List(Vec<TemplateValueRepr>) and the corresponding
    // List(TemplateValueType) descriptor. Until then, a list is represented as
    // a non-optional opaque value.
    OpaqueList(Rc<dyn Any>),
}

impl TemplateValueRepr {
    /// Identifies the underlying value type.
    ///
    /// For types supporting `Option<T>`, this returns `Option(T)` instead of
    /// `Scalar(T)`.
    pub fn actual_value_type(&self) -> TemplateValueType {
        match &self.tagged {
            // Tagged::ByteString(_) => BString::value_type(),
            // Tagged::String(_) => String::value_type(),
            // Tagged::Boolean(_) => bool::value_type(),
            // Tagged::Integer(_) => <Option<i64>>::value_type(),
            Tagged::OpaqueSome(opaque) => TemplateValueType::Option(Rc::as_ref(opaque).type_id()),
            Tagged::OpaqueNone(id) => TemplateValueType::Option(*id),
            Tagged::OpaqueList(opaque) => TemplateValueType::Scalar(Rc::as_ref(opaque).type_id()),
        }
    }
}

macro_rules! impl_inline_scalar_values {
    ($( $var:ident($ty:ty) ),* $(,)?) => {$(
        impl TemplateValue for $ty {
            fn value_type() -> TemplateValueType {
                TemplateValueType::Scalar(TypeId::of::<$ty>())
            }

            fn try_from_repr(
                repr: TemplateValueRepr,
            ) -> Result<Self, TemplateValueExtractionError> {
                match repr.tagged {
                    Tagged::$var(value) => Ok(value),
                    _ => Err(unexpected_type(&repr)),
                }
            }

            fn into_repr(self) -> TemplateValueRepr {
                let tagged = Tagged::$var(self);
                TemplateValueRepr { tagged }
            }
        }
    )*};
}

macro_rules! impl_inline_optional_values {
    ($( $var:ident($ty:ty) ),* $(,)?) => {$(
        impl TemplateValue for $ty {
            fn value_type() -> TemplateValueType {
                TemplateValueType::Scalar(TypeId::of::<$ty>())
            }

            fn try_from_repr(
                repr: TemplateValueRepr,
            ) -> Result<Self, TemplateValueExtractionError> {
                <Option<$ty>>::try_from_repr(repr)?
                    .ok_or(TemplateValueExtractionError::MissingValue)
            }

            fn into_repr(self) -> TemplateValueRepr {
                let tagged = Tagged::$var(Some(self));
                TemplateValueRepr { tagged }
            }
        }

        impl TemplateValue for Option<$ty> {
            fn value_type() -> TemplateValueType {
                TemplateValueType::Option(TypeId::of::<$ty>())
            }

            fn try_from_repr(
                repr: TemplateValueRepr,
            ) -> Result<Self, TemplateValueExtractionError> {
                match repr.tagged {
                    Tagged::$var(optional) => Ok(optional),
                    _ => Err(unexpected_type(&repr)),
                }
            }

            fn into_repr(self) -> TemplateValueRepr {
                let tagged = Tagged::$var(self);
                TemplateValueRepr { tagged }
            }
        }
    )*};
}

impl_inline_scalar_values! {
    // ByteString(BString),
    // String(String),
    // Boolean(bool),
}

impl_inline_optional_values! {
    // Integer(i64),
}

impl<T: OpaqueTemplateValue> TemplateValue for T {
    fn value_type() -> TemplateValueType {
        TemplateValueType::Scalar(TypeId::of::<T>())
    }

    fn try_from_repr(repr: TemplateValueRepr) -> Result<Self, TemplateValueExtractionError> {
        <Option<T>>::try_from_repr(repr)?.ok_or(TemplateValueExtractionError::MissingValue)
    }

    fn into_repr(self) -> TemplateValueRepr {
        let tagged = Tagged::OpaqueSome(Rc::new(self));
        TemplateValueRepr { tagged }
    }
}

impl<T: OpaqueTemplateValue> TemplateValue for Option<T> {
    fn value_type() -> TemplateValueType {
        TemplateValueType::Option(TypeId::of::<T>())
    }

    fn try_from_repr(repr: TemplateValueRepr) -> Result<Self, TemplateValueExtractionError> {
        match repr.tagged {
            Tagged::OpaqueSome(opaque) => {
                let value = downcast_value::<T>(opaque).map_err(|opaque| {
                    let tagged = Tagged::OpaqueSome(opaque);
                    unexpected_type(&TemplateValueRepr { tagged })
                })?;
                Ok(Some(value))
            }
            Tagged::OpaqueNone(id) if id == TypeId::of::<T>() => Ok(None),
            _ => Err(unexpected_type(&repr)),
        }
    }

    fn into_repr(self) -> TemplateValueRepr {
        let tagged = match self {
            Some(value) => Tagged::OpaqueSome(Rc::new(value)),
            None => Tagged::OpaqueNone(TypeId::of::<T>()),
        };
        TemplateValueRepr { tagged }
    }
}

macro_rules! impl_opaque_list_values {
    ($( <$($gen:ident : $bound:path),*> $ty:ty; )*) => {$(
        impl<$($gen: $bound),*> TemplateValue for $ty {
            fn value_type() -> TemplateValueType {
                TemplateValueType::Scalar(TypeId::of::<$ty>())
            }

            fn try_from_repr(
                repr: TemplateValueRepr,
            ) -> Result<Self, TemplateValueExtractionError> {
                match repr.tagged {
                    Tagged::OpaqueList(opaque) => downcast_value::<$ty>(opaque).map_err(|opaque| {
                        let tagged = Tagged::OpaqueList(opaque);
                        unexpected_type(&TemplateValueRepr { tagged })
                    }),
                    _ => Err(unexpected_type(&repr)),
                }
            }

            fn into_repr(self) -> TemplateValueRepr {
                let tagged = Tagged::OpaqueList(Rc::new(self));
                TemplateValueRepr { tagged }
            }
        }
    )*};
}

impl_opaque_list_values! {
    // <> Vec<BString>;
    // <> Vec<String>;
    <T: OpaqueTemplateValue> Vec<T>;
    <T: OpaqueTemplateValue> Vec<Option<T>>;
}

fn downcast_value<T: Clone + 'static>(opaque: Rc<dyn Any>) -> Result<T, Rc<dyn Any>> {
    let value = Rc::downcast(opaque)?;
    Ok(Rc::unwrap_or_clone(value))
}

fn unexpected_type(repr: &TemplateValueRepr) -> TemplateValueExtractionError {
    TemplateValueExtractionError::UnexpectedType(repr.actual_value_type())
}

pub trait ToTemplateCondition {
    fn to_condition(&self) -> bool;
}

impl ToTemplateCondition for BString {
    fn to_condition(&self) -> bool {
        !self.is_empty()
    }
}

impl ToTemplateCondition for String {
    fn to_condition(&self) -> bool {
        !self.is_empty()
    }
}

impl ToTemplateCondition for bool {
    fn to_condition(&self) -> bool {
        *self
    }
}

impl<T> ToTemplateCondition for Option<T> {
    fn to_condition(&self) -> bool {
        self.is_some()
    }
}

impl<T> ToTemplateCondition for Vec<T> {
    fn to_condition(&self) -> bool {
        !self.is_empty()
    }
}

type CastFn<T> = fn(TemplateValueRepr) -> Result<T, TemplateValueExtractionError>;
type BinaryFn<T> =
    fn(TemplateValueRepr, TemplateValueRepr) -> Result<T, TemplateValueExtractionError>;
type ToConditionFn = CastFn<bool>;
type ToTemplateFn = CastFn<Box<dyn Template>>;
type ToSerializeFn = CastFn<Box<dyn erased_serde::Serialize>>;
type EqFn = BinaryFn<bool>;
type CmpFn = BinaryFn<Ordering>;

pub struct TemplateTypeRegistry {
    type_names: HashMap<TemplateValueType, &'static str>,
    to_condition_fns: HashMap<TemplateValueType, ToConditionFn>,
    to_template_fns: HashMap<TemplateValueType, ToTemplateFn>,
    to_serialize_fns: HashMap<TemplateValueType, ToSerializeFn>,
    eq_fns: HashMap<TemplateValueType, HashMap<TemplateValueType, EqFn>>,
    cmp_fns: HashMap<TemplateValueType, CmpFn>,
}

impl TemplateTypeRegistry {
    pub fn empty() -> Self {
        Self {
            type_names: HashMap::new(),
            to_condition_fns: HashMap::new(),
            to_template_fns: HashMap::new(),
            to_serialize_fns: HashMap::new(),
            eq_fns: HashMap::new(),
            cmp_fns: HashMap::new(),
        }
    }

    /// Registers a new template value type.
    #[track_caller]
    pub fn register<T: TemplateValue>(
        &mut self,
        type_name: &'static str,
    ) -> TemplateTypeRegistration<'_, T> {
        let old_type_name = self.type_names.insert(T::value_type(), type_name);
        assert_eq!(old_type_name, None, "type already registered: {type_name}");
        TemplateTypeRegistration {
            registry: self,
            ty: PhantomData,
        }
    }

    pub fn get_type_name(&self, ty: &TemplateValueType) -> Option<&'static str> {
        self.type_names.get(ty).copied()
    }

    pub fn get_to_condition_fn(&self, ty: &TemplateValueType) -> Option<ToConditionFn> {
        self.to_condition_fns.get(ty).copied()
    }

    pub fn get_to_template_fn(&self, ty: &TemplateValueType) -> Option<ToTemplateFn> {
        self.to_template_fns.get(ty).copied()
    }

    pub fn get_to_serialize_fn(&self, ty: &TemplateValueType) -> Option<ToSerializeFn> {
        self.to_serialize_fns.get(ty).copied()
    }

    pub fn get_eq_fn(&self, ty1: &TemplateValueType, ty2: &TemplateValueType) -> Option<EqFn> {
        self.eq_fns.get(ty1)?.get(ty2).copied()
    }

    pub fn get_cmp_fn(&self, ty1: &TemplateValueType, ty2: &TemplateValueType) -> Option<CmpFn> {
        (ty1 == ty2)
            .then(|| self.cmp_fns.get(ty1).copied())
            .flatten()
    }
}

pub struct TemplateTypeRegistration<'a, T> {
    registry: &'a mut TemplateTypeRegistry,
    ty: PhantomData<fn(T)>,
}

impl<T: TemplateValue> TemplateTypeRegistration<'_, T> {
    /// Registers this as a type convertible to boolean.
    pub fn conditionable(self) -> Self
    where
        T: ToTemplateCondition,
    {
        self.registry
            .to_condition_fns
            .insert(T::value_type(), try_into_condition::<T>);
        self
    }

    /// Registers this as a formattable type.
    pub fn formattable(self) -> Self
    where
        T: Template,
    {
        self.registry
            .to_template_fns
            .insert(T::value_type(), try_into_template::<T>);
        self
    }

    /// Registers this as a serializable type.
    pub fn serializable(self) -> Self
    where
        T: serde::Serialize,
    {
        self.registry
            .to_serialize_fns
            .insert(T::value_type(), try_into_serialize::<T>);
        self
    }

    /// Registers this as an equality-comparable type.
    pub fn equatable(self) -> Self
    where
        T: Eq,
    {
        self.registry
            .eq_fns
            .entry(T::value_type())
            .or_default()
            .insert(T::value_type(), try_eq::<T, T>);
        self
    }

    /// Registers this and other as equality-comparable types.
    ///
    /// The comparison function is registered for both `self == other` and
    /// `other == self`, allowing core types to be compared with non-core types.
    pub fn equatable_with<U>(self) -> Self
    where
        T: PartialEq<U>,
        U: PartialEq<T> + TemplateValue,
    {
        self.registry
            .eq_fns
            .entry(T::value_type())
            .or_default()
            .insert(U::value_type(), try_eq::<T, U>);
        self.registry
            .eq_fns
            .entry(U::value_type())
            .or_default()
            .insert(T::value_type(), try_eq::<U, T>);
        self
    }

    /// Registers this as an order-comparable type.
    pub fn comparable(self) -> Self
    where
        T: Ord,
    {
        self.registry.cmp_fns.insert(T::value_type(), try_cmp::<T>);
        self
    }
}

fn try_into_condition<T: TemplateValue + ToTemplateCondition>(
    repr: TemplateValueRepr,
) -> Result<bool, TemplateValueExtractionError> {
    let value = T::try_from_repr(repr)?;
    Ok(value.to_condition())
}

fn try_into_template<T: TemplateValue + Template>(
    repr: TemplateValueRepr,
) -> Result<Box<dyn Template>, TemplateValueExtractionError> {
    let value = T::try_from_repr(repr)?;
    Ok(Box::new(value))
}

fn try_into_serialize<T: TemplateValue + serde::Serialize>(
    repr: TemplateValueRepr,
) -> Result<Box<dyn erased_serde::Serialize>, TemplateValueExtractionError> {
    let value = T::try_from_repr(repr)?;
    Ok(Box::new(value))
}

fn try_eq<T: TemplateValue + PartialEq<U>, U: TemplateValue>(
    repr1: TemplateValueRepr,
    repr2: TemplateValueRepr,
) -> Result<bool, TemplateValueExtractionError> {
    let value1 = T::try_from_repr(repr1)?;
    let value2 = U::try_from_repr(repr2)?;
    Ok(value1 == value2)
}

fn try_cmp<T: TemplateValue + Ord>(
    repr1: TemplateValueRepr,
    repr2: TemplateValueRepr,
) -> Result<Ordering, TemplateValueExtractionError> {
    let value1 = T::try_from_repr(repr1)?;
    let value2 = T::try_from_repr(repr2)?;
    Ok(value1.cmp(&value2))
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    #[derive(Clone, Debug)]
    struct Value(i32);
    #[derive(Clone, Debug)]
    struct Other;

    impl OpaqueTemplateValue for Value {}
    impl OpaqueTemplateValue for Other {}

    #[test]
    fn test_opaque_values() {
        let scalar_value = Value(42).into_repr();
        assert_matches!(Value::try_from_repr(scalar_value.clone()), Ok(Value(42)));
        assert_matches!(
            Option::<Value>::try_from_repr(scalar_value.clone()),
            Ok(Some(Value(42)))
        );

        let none_value = None::<Value>.into_repr();
        assert_matches!(
            Value::try_from_repr(none_value.clone()),
            Err(TemplateValueExtractionError::MissingValue)
        );
        assert_matches!(Option::<Value>::try_from_repr(none_value.clone()), Ok(None));

        let some_value = Some(Value(42)).into_repr();
        assert_matches!(Value::try_from_repr(some_value.clone()), Ok(Value(42)));
        assert_matches!(
            Option::<Value>::try_from_repr(some_value.clone()),
            Ok(Some(Value(42)))
        );

        let some_other = Some(Other).into_repr();
        assert_matches!(
            Value::try_from_repr(some_other.clone()),
            Err(TemplateValueExtractionError::UnexpectedType(ty))
                if ty == <Option<Other>>::value_type()
        );
        assert_matches!(
            Option::<Value>::try_from_repr(some_other.clone()),
            Err(TemplateValueExtractionError::UnexpectedType(ty))
                if ty == <Option<Other>>::value_type()
        );

        let none_other = None::<Other>.into_repr();
        assert_matches!(
            Value::try_from_repr(none_other.clone()),
            Err(TemplateValueExtractionError::UnexpectedType(ty))
                if ty == <Option<Other>>::value_type()
        );
        assert_matches!(
            Option::<Value>::try_from_repr(none_other.clone()),
            Err(TemplateValueExtractionError::UnexpectedType(ty))
                if ty == <Option<Other>>::value_type()
        );
    }
}
