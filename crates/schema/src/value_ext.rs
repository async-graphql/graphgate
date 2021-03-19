use indexmap::IndexSet;
use value::Value;

pub trait ValueExt {
    fn referenced_variables(&self) -> IndexSet<&str>;
}

impl ValueExt for Value {
    fn referenced_variables(&self) -> IndexSet<&str> {
        pub fn referenced_variables_to_set<'a>(value: &'a Value, vars: &mut IndexSet<&'a str>) {
            match value {
                Value::Variable(name) => {
                    vars.insert(name);
                }
                Value::List(values) => values
                    .iter()
                    .for_each(|value| referenced_variables_to_set(value, vars)),
                Value::Object(obj) => obj
                    .values()
                    .for_each(|value| referenced_variables_to_set(value, vars)),
                _ => {}
            }
        }

        let mut vars = IndexSet::new();
        referenced_variables_to_set(self, &mut vars);
        vars
    }
}
