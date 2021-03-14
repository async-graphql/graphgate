use indexmap::IndexSet;

use value::Value;

pub fn referenced_variables(value: &Value) -> IndexSet<&str> {
    pub fn referenced_variables_to_hashset<'a>(value: &'a Value, vars: &mut IndexSet<&'a str>) {
        match value {
            Value::Variable(name) => {
                vars.insert(name);
            }
            Value::List(values) => values
                .iter()
                .for_each(|value| referenced_variables_to_hashset(value, vars)),
            Value::Object(obj) => obj
                .values()
                .for_each(|value| referenced_variables_to_hashset(value, vars)),
            _ => {}
        }
    }

    let mut vars = IndexSet::new();
    referenced_variables_to_hashset(value, &mut vars);
    vars
}
