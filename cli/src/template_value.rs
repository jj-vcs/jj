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
use std::rc::Rc;

use bstr::BString;

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
    ByteString(BString),
    String(String),
    Boolean(bool),
    Integer(Option<i64>),
    // Opaque value, or its type if no value is available:
    OpaqueSome(Rc<dyn Any>),
    OpaqueNone(TypeId),
}

impl TemplateValueRepr {
    /// Identifies the underlying value type.
    ///
    /// For types supporting `Option<T>`, this returns `Option(T)` instead of
    /// `Scalar(T)`.
    pub fn actual_value_type(&self) -> TemplateValueType {
        match &self.tagged {
            Tagged::ByteString(_) => BString::value_type(),
            Tagged::String(_) => String::value_type(),
            Tagged::Boolean(_) => bool::value_type(),
            Tagged::Integer(_) => <Option<i64>>::value_type(),
            Tagged::OpaqueSome(opaque) => TemplateValueType::Option(Rc::as_ref(opaque).type_id()),
            Tagged::OpaqueNone(id) => TemplateValueType::Option(*id),
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
    ByteString(BString),
    String(String),
    Boolean(bool),
}

impl_inline_optional_values! {
    Integer(i64),
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

fn downcast_value<T: Clone + 'static>(opaque: Rc<dyn Any>) -> Result<T, Rc<dyn Any>> {
    let value = Rc::downcast(opaque)?;
    Ok(Rc::unwrap_or_clone(value))
}

fn unexpected_type(repr: &TemplateValueRepr) -> TemplateValueExtractionError {
    TemplateValueExtractionError::UnexpectedType(repr.actual_value_type())
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
