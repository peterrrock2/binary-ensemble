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
