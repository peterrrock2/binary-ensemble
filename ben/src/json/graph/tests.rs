use super::*;
use serde_json::Value;

#[test]
fn test_relabel_small_file() {
    let input = r#"{
    "adjacency": [
        [ { "id": 3 }, { "id": 1 } ],
        [ { "id": 0 }, { "id": 4 }, { "id": 2 } ],
        [ { "id": 1 }, { "id": 5 } ],
        [ { "id": 0 }, { "id": 6 }, { "id": 4 } ],
        [ { "id": 1 }, { "id": 3 }, { "id": 7 }, { "id": 5 } ],
        [ { "id": 2 }, { "id": 4 }, { "id": 8 } ],
        [ { "id": 3 }, { "id": 7 } ],
        [ { "id": 4 }, { "id": 6 }, { "id": 8 } ],
        [ { "id": 5 }, { "id": 7 } ]
    ],
    "directed": false,
    "graph": [],
    "multigraph": false,
    "nodes": [
        {
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "GEOID20": "20258288005",
            "id": 0
        },
        {
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "GEOID20": "20258288004",
            "id": 1
        },
        {
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "GEOID20": "20258288003",
            "id": 2
        },
        {
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "GEOID20": "20258288006",
            "id": 3
        },
        {
            "TOTPOP": 1,
            "boundary_nodes": false,
            "boundary_perim": 0,
            "GEOID20": "20258288001",
            "id": 4
        },
        {
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "GEOID20": "20258288002",
            "id": 5
        },
        {
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "GEOID20": "20258288007",
            "id": 6
        },
        {
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "GEOID20": "20258288008",
            "id": 7
        },
        {
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "GEOID20": "20258288009",
            "id": 8
        }
    ]
}
"#;

    let reader = input.as_bytes();

    let mut output = Vec::new();
    let writer = &mut output;

    let key = "GEOID20";

    let _ = sort_json_file_by_key(reader, writer, key).unwrap();

    let expected_output = r#"{
    "adjacency": [
        [ { "id": 3 }, { "id": 5 }, { "id": 7 }, { "id": 1 } ],
        [ { "id": 2 }, { "id": 0 }, { "id": 8 } ], [ { "id": 3 }, { "id": 1 } ],
        [ { "id": 4 }, { "id": 0 }, { "id": 2 } ],
        [ { "id": 5 }, { "id": 3 } ],
        [ { "id": 4 }, { "id": 6 }, { "id": 0 } ],
        [ { "id": 5 }, { "id": 7 } ],
        [ { "id": 0 }, { "id": 6 }, { "id": 8 } ],
        [ { "id": 1 }, { "id": 7 } ]
    ],
    "directed": false,
    "graph": [],
    "multigraph": false,
    "nodes": [
        {
            "GEOID20": "20258288001",
            "TOTPOP": 1,
            "boundary_nodes": false,
            "boundary_perim": 0,
            "id": 0
        },
        {
            "GEOID20": "20258288002",
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "id": 1
        },
        {
            "GEOID20": "20258288003",
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "id": 2
        },
        {
            "GEOID20": "20258288004",
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "id": 3
        },
        {
            "GEOID20": "20258288005",
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "id": 4
        },
        {
            "GEOID20": "20258288006",
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "id": 5
        },
        {
            "GEOID20": "20258288007",
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "id": 6
        },
        {
            "GEOID20": "20258288008",
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "id": 7
        },
        {
            "GEOID20": "20258288009",
            "TOTPOP": 1,
            "boundary_nodes": true,
            "boundary_perim": 1,
            "id": 8
        }
    ]
}
"#;

    let output_json: Value = serde_json::from_slice(&output).unwrap();
    let expected_output_json: Value = serde_json::from_str(expected_output).unwrap();

    assert_eq!(output_json, expected_output_json);
}

#[test]
fn test_sort_json_file_by_numeric_key() {
    let input = r#"{
        "nodes": [
            {"id": 0, "rank": 20},
            {"id": 1, "rank": 5},
            {"id": 2, "rank": 10}
        ],
        "adjacency": [
            [{"id": 1}],
            [{"id": 0}, {"id": 2}],
            [{"id": 1}]
        ]
    }"#;

    let mut output = Vec::new();
    let mapping = sort_json_file_by_key(input.as_bytes(), &mut output, "rank").unwrap();
    let output_json: Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(mapping.get(&1), Some(&0));
    assert_eq!(mapping.get(&2), Some(&1));
    assert_eq!(mapping.get(&0), Some(&2));
    assert_eq!(output_json["nodes"][0]["rank"], 5);
    assert_eq!(output_json["nodes"][1]["rank"], 10);
    assert_eq!(output_json["nodes"][2]["rank"], 20);
}

#[test]
fn test_sort_json_file_by_key_with_non_numeric_values() {
    let input = r#"{
        "nodes": [
            {"id": 0, "key": {"nested": true}},
            {"id": 1, "key": "abc"},
            {"id": 2, "key": 7}
        ],
        "adjacency": [
            [{"id": 1}],
            [{"id": 2}],
            [{"id": 0}]
        ]
    }"#;

    let mut output = Vec::new();
    sort_json_file_by_key(input.as_bytes(), &mut output, "key").unwrap();
    let output_json: Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(output_json["nodes"][0]["key"], 7);
    assert_eq!(output_json["nodes"][1]["key"], "abc");
    assert_eq!(output_json["nodes"][2]["key"], serde_json::json!({"nested": true}));
}

#[test]
fn test_sort_json_file_by_key_err_string_vs_number_branch() {
    let input = r#"{
        "nodes": [
            {"id": 0, "key": "zzz"},
            {"id": 1, "key": 3}
        ],
        "adjacency": [
            [{"id": 1}],
            [{"id": 0}]
        ]
    }"#;

    let mut output = Vec::new();
    sort_json_file_by_key(input.as_bytes(), &mut output, "key").unwrap();
    let output_json: Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(output_json["nodes"][0]["key"], 3);
    assert_eq!(output_json["nodes"][1]["key"], "zzz");
}

#[test]
fn test_sort_json_file_by_key_without_nodes_or_edges() {
    let input = br#"{"graph": [], "directed": false}"#;
    let mut output = Vec::new();

    let mapping = sort_json_file_by_key(&input[..], &mut output, "unused").unwrap();
    let output_json: Value = serde_json::from_slice(&output).unwrap();

    assert!(mapping.is_empty());
    assert_eq!(output_json["graph"], serde_json::json!([]));
    assert_eq!(output_json["directed"], false);
}
