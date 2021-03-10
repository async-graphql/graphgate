use graphgate_core::{ComposedSchema, PlanBuilder};

#[test]
fn test_subscription() {
    let schema = ComposedSchema::parse(include_str!("test.graphql")).unwrap();
    let query = r#"
        subscription {
          users {
            username
            reviews {
              body
            }
          }
        }
    "#;
    let doc = parser::parse_query(query).unwrap();
    let builder = PlanBuilder::new(&schema, doc);
    let plan = serde_json::to_string_pretty(&builder.plan_subscribe().unwrap()).unwrap();
    println!("{}", plan);
}
