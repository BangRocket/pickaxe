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
    #[serde(default)]
    hardness: f64,
    #[serde(default)]
    diggable: bool,
    #[serde(rename = "harvestTools")]
    harvest_tools: Option<HashMap<String, bool>>,
}

#[derive(Deserialize)]
struct Item {
    id: i32,
    name: String,
    #[serde(rename = "stackSize")]
    stack_size: i32,
}

/// Load all JSON files from a directory, deserialize as Vec<T>, merge, and sort by ID.
fn load_from_dir<T: serde::de::DeserializeOwned>(dir: &Path, id_fn: fn(&T) -> i32) -> Vec<T> {
    let mut all = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("Cannot read directory {:?}: {}", dir, e))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let contents = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Cannot read {:?}: {}", path, e));
        let items: Vec<T> = serde_json::from_str(&contents)
            .unwrap_or_else(|e| panic!("Invalid JSON in {:?}: {}", path, e));
        all.extend(items);
    }
    all.sort_by_key(|item| id_fn(item));
    all
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

    let blocks_dir = data_dir.join("blocks");
    let items_dir = data_dir.join("items");

    let blocks: Vec<Block> = load_from_dir(&blocks_dir, |b| b.id);
    let items: Vec<Item> = load_from_dir(&items_dir, |i| i.id);

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
    writeln!(out).unwrap();

    // block_state_to_hardness
    writeln!(
        out,
        "/// Map block state ID to (hardness, diggable)."
    )
    .unwrap();
    writeln!(
        out,
        "pub fn block_state_to_hardness(state_id: i32) -> Option<(f64, bool)> {{"
    )
    .unwrap();
    writeln!(out, "    match state_id {{").unwrap();
    for b in &blocks {
        let diggable = if b.diggable { "true" } else { "false" };
        if b.min_state_id == b.max_state_id {
            writeln!(
                out,
                "        {} => Some(({:?}, {})), // {}",
                b.min_state_id, b.hardness, diggable, b.name
            )
            .unwrap();
        } else {
            writeln!(
                out,
                "        {}..={} => Some(({:?}, {})), // {}",
                b.min_state_id, b.max_state_id, b.hardness, diggable, b.name
            )
            .unwrap();
        }
    }
    writeln!(out, "        _ => None,").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // block_state_to_drops
    writeln!(
        out,
        "/// Map block state ID to dropped item IDs."
    )
    .unwrap();
    writeln!(
        out,
        "pub fn block_state_to_drops(state_id: i32) -> &'static [i32] {{"
    )
    .unwrap();
    writeln!(out, "    match state_id {{").unwrap();
    for b in &blocks {
        if b.drops.is_empty() {
            continue;
        }
        let drops_str: Vec<String> = b.drops.iter().map(|d| d.to_string()).collect();
        let drops_list = drops_str.join(", ");
        if b.min_state_id == b.max_state_id {
            writeln!(
                out,
                "        {} => &[{}], // {}",
                b.min_state_id, drops_list, b.name
            )
            .unwrap();
        } else {
            writeln!(
                out,
                "        {}..={} => &[{}], // {}",
                b.min_state_id, b.max_state_id, drops_list, b.name
            )
            .unwrap();
        }
    }
    writeln!(out, "        _ => &[],").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // block_state_to_harvest_tools
    writeln!(
        out,
        "/// Map block state ID to required harvest tool IDs (None = any tool works)."
    )
    .unwrap();
    writeln!(
        out,
        "pub fn block_state_to_harvest_tools(state_id: i32) -> Option<&'static [i32]> {{"
    )
    .unwrap();
    writeln!(out, "    match state_id {{").unwrap();
    for b in &blocks {
        if let Some(ref tools) = b.harvest_tools {
            let mut tool_ids: Vec<i32> = tools.keys().filter_map(|k| k.parse::<i32>().ok()).collect();
            tool_ids.sort();
            let tools_str: Vec<String> = tool_ids.iter().map(|id| id.to_string()).collect();
            let tools_list = tools_str.join(", ");
            if b.min_state_id == b.max_state_id {
                writeln!(
                    out,
                    "        {} => Some(&[{}]), // {}",
                    b.min_state_id, tools_list, b.name
                )
                .unwrap();
            } else {
                writeln!(
                    out,
                    "        {}..={} => Some(&[{}]), // {}",
                    b.min_state_id, b.max_state_id, tools_list, b.name
                )
                .unwrap();
            }
        } else {
            if b.min_state_id == b.max_state_id {
                writeln!(
                    out,
                    "        {} => None, // {}",
                    b.min_state_id, b.name
                )
                .unwrap();
            } else {
                writeln!(
                    out,
                    "        {}..={} => None, // {}",
                    b.min_state_id, b.max_state_id, b.name
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
        blocks_dir.display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        items_dir.display()
    );
}
