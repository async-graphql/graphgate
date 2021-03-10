use std::collections::HashSet;

use value::Value;

pub fn referenced_variables(value: &Value) -> HashSet<&str> {
    pub fn referenced_variables_to_hashset<'a>(value: &'a Value, vars: &mut HashSet<&'a str>) {
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

    let mut vars = HashSet::new();
    referenced_variables_to_hashset(value, &mut vars);
    vars
}
