use std::fs;

use globset::GlobBuilder;
use graphgate_core::{ComposedSchema, PlanBuilder};

#[test]
fn test() {
    let schema = ComposedSchema::parse(include_str!("test.graphql")).unwrap();
    let glob = GlobBuilder::new("./tests/*.txt")
        .literal_separator(true)
        .build()
        .unwrap()
        .compile_matcher();

    for entry in fs::read_dir("./tests").unwrap() {
        let entry = entry.unwrap();
        if !glob.is_match(entry.path()) {
            continue;
        }

        println!("{}", entry.path().display());

        let data = fs::read_to_string(&entry.path()).unwrap();
        let mut s = data.split("---");
        let mut n = 1;

        loop {
            println!("\tIndex: {}", n);
            let graphql = match s.next() {
                Some(graphql) => graphql,
                None => break,
            };
            let planner_json = s.next().unwrap();

            let document = parser::parse_query(graphql).unwrap();
            let builder = PlanBuilder::new(&schema, document);
            let expect_node: serde_json::Value = serde_json::from_str(planner_json).unwrap();
            let actual_node = serde_json::to_value(&builder.plan().unwrap()).unwrap();

            // assert_eq!(
            //     serde_json::to_string_pretty(&actual_node).unwrap(),
            //     serde_json::to_string_pretty(&expect_node).unwrap(),
            // );
            assert_eq!(actual_node, expect_node);

            n += 1;
        }
    }
}
