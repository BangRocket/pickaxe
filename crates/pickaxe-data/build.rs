use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;

#[derive(Deserialize)]
#[allow(dead_code)]
struct Block {
    id: i32,
    name: String,
    #[serde(rename = "minStateId")]
    min_state_id: i32,
    #[serde(rename = "maxStateId")]
    max_state_id: i32,
    #[serde(rename = "defaultState")]
    default_state: i32,
    #[serde(default)]
    drops: Vec<i32>,
}

#[derive(Deserialize)]
struct Item {
    id: i32,
    name: String,
    #[serde(rename = "stackSize")]
    stack_size: i32,
}

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let data_dir = Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("data/minecraft");
    let out_dir = std::env::var("OUT_DIR").unwrap();

    let blocks_json = fs::read_to_string(data_dir.join("blocks.json"))
        .expect("Missing data/minecraft/blocks.json");
    let items_json = fs::read_to_string(data_dir.join("items.json"))
        .expect("Missing data/minecraft/items.json");

    let blocks: Vec<Block> = serde_json::from_str(&blocks_json).unwrap();
    let items: Vec<Item> = serde_json::from_str(&items_json).unwrap();

    let item_by_name: HashMap<&str, &Item> = items.iter().map(|i| (i.name.as_str(), i)).collect();

    let mut out = fs::File::create(Path::new(&out_dir).join("generated.rs")).unwrap();

    // block_name_to_default_state
    writeln!(out, "/// Map block name to its default block state ID.").unwrap();
    writeln!(
        out,
        "pub fn block_name_to_default_state(name: &str) -> Option<i32> {{"
    )
    .unwrap();
    writeln!(out, "    match name {{").unwrap();
    for b in &blocks {
        writeln!(
            out,
            "        \"{}\" => Some({}),",
            b.name, b.default_state
        )
        .unwrap();
    }
    writeln!(out, "        _ => None,").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // item_name_to_id
    writeln!(out, "/// Map item name to item registry ID.").unwrap();
    writeln!(
        out,
        "pub fn item_name_to_id(name: &str) -> Option<i32> {{"
    )
    .unwrap();
    writeln!(out, "    match name {{").unwrap();
    for i in &items {
        writeln!(out, "        \"{}\" => Some({}),", i.name, i.id).unwrap();
    }
    writeln!(out, "        _ => None,").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // item_id_to_name
    writeln!(out, "/// Map item ID to item name.").unwrap();
    writeln!(
        out,
        "pub fn item_id_to_name(id: i32) -> Option<&'static str> {{"
    )
    .unwrap();
    writeln!(out, "    match id {{").unwrap();
    for i in &items {
        writeln!(out, "        {} => Some(\"{}\"),", i.id, i.name).unwrap();
    }
    writeln!(out, "        _ => None,").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // item_id_to_stack_size
    writeln!(out, "/// Map item ID to max stack size.").unwrap();
    writeln!(
        out,
        "pub fn item_id_to_stack_size(id: i32) -> Option<i32> {{"
    )
    .unwrap();
    writeln!(out, "    match id {{").unwrap();
    for i in &items {
        writeln!(out, "        {} => Some({}),", i.id, i.stack_size).unwrap();
    }
    writeln!(out, "        _ => None,").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // item_id_to_block_state
    writeln!(
        out,
        "/// Map item ID to the default block state it places (if it's a block item)."
    )
    .unwrap();
    writeln!(
        out,
        "pub fn item_id_to_block_state(item_id: i32) -> Option<i32> {{"
    )
    .unwrap();
    writeln!(out, "    match item_id {{").unwrap();
    for b in &blocks {
        if let Some(item) = item_by_name.get(b.name.as_str()) {
            writeln!(
                out,
                "        {} => Some({}), // {}",
                item.id, b.default_state, b.name
            )
            .unwrap();
        }
    }
    writeln!(out, "        _ => None,").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // block_state_to_item_id
    writeln!(
        out,
        "/// Map any block state ID to the item ID it drops/represents."
    )
    .unwrap();
    writeln!(
        out,
        "pub fn block_state_to_item_id(state_id: i32) -> Option<i32> {{"
    )
    .unwrap();
    writeln!(out, "    match state_id {{").unwrap();
    for b in &blocks {
        let item_id = if !b.drops.is_empty() {
            Some(b.drops[0])
        } else {
            item_by_name.get(b.name.as_str()).map(|i| i.id)
        };
        if let Some(iid) = item_id {
            if b.min_state_id == b.max_state_id {
                writeln!(
                    out,
                    "        {} => Some({}), // {}",
                    b.min_state_id, iid, b.name
                )
                .unwrap();
            } else {
                writeln!(
                    out,
                    "        {}..={} => Some({}), // {}",
                    b.min_state_id, b.max_state_id, iid, b.name
                )
                .unwrap();
            }
        }
    }
    writeln!(out, "        _ => None,").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();

    println!(
        "cargo:rerun-if-changed={}",
        data_dir.join("blocks.json").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        data_dir.join("items.json").display()
    );
}
