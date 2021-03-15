use parser::types::{BaseType, Type};

pub trait TypeExt {
    fn concrete_typename(&self) -> &str;
    fn is_subtype(&self, sub: &Type) -> bool;
}

impl TypeExt for Type {
    fn concrete_typename(&self) -> &str {
        match &self.base {
            BaseType::Named(name) => name.as_str(),
            BaseType::List(ty) => ty.concrete_typename(),
        }
    }

    fn is_subtype(&self, sub: &Type) -> bool {
        if !sub.nullable || self.nullable {
            match (&self.base, &sub.base) {
                (BaseType::Named(super_type), BaseType::Named(sub_type)) => super_type == sub_type,
                (BaseType::List(super_type), BaseType::List(sub_type)) => {
                    super_type.is_subtype(sub_type)
                }
                _ => false,
            }
        } else {
            false
        }
    }
}
