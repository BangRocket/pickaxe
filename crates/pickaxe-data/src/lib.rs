include!(concat!(env!("OUT_DIR"), "/generated.rs"));

/// Returns the fuel burn time in ticks for the given item, or None if it is not a fuel.
pub fn fuel_burn_time(item_id: i32) -> Option<i16> {
    let name = item_id_to_name(item_id)?;
    match name {
        "coal" | "charcoal" => Some(1600),
        "oak_log" | "spruce_log" | "birch_log" | "jungle_log" | "acacia_log"
        | "dark_oak_log" => Some(300),
        "oak_planks" | "spruce_planks" | "birch_planks" | "jungle_planks"
        | "acacia_planks" | "dark_oak_planks" => Some(300),
        "stick" => Some(100),
        "coal_block" => Some(16000),
        _ => None,
    }
}

/// Returns (result_item_id, cook_time_ticks) for items that can be smelted, or None.
pub fn smelting_result(item_id: i32) -> Option<(i32, i16)> {
    let name = item_id_to_name(item_id)?;
    let (result_name, cook_time) = match name {
        "cobblestone" => ("stone", 200),
        "sand" => ("glass", 200),
        "iron_ore" | "raw_iron" => ("iron_ingot", 200),
        "gold_ore" | "raw_gold" => ("gold_ingot", 200),
        "oak_log" | "spruce_log" | "birch_log" | "jungle_log" | "acacia_log"
        | "dark_oak_log" => ("charcoal", 200),
        "clay_ball" => ("brick", 200),
        "beef" => ("cooked_beef", 200),
        "porkchop" => ("cooked_porkchop", 200),
        "chicken" => ("cooked_chicken", 200),
        "mutton" => ("cooked_mutton", 200),
        "rabbit" => ("cooked_rabbit", 200),
        "cod" => ("cooked_cod", 200),
        "salmon" => ("cooked_salmon", 200),
        "potato" => ("baked_potato", 200),
        _ => return None,
    };
    let result_id = item_name_to_id(result_name)?;
    Some((result_id, cook_time))
}

/// Food properties for edible items.
pub struct FoodProperties {
    pub nutrition: i32,
    pub saturation_modifier: f32,
    pub eat_ticks: i32,
    pub can_always_eat: bool,
}

/// Returns food properties for the given item, or None if it is not edible.
pub fn food_properties(item_id: i32) -> Option<FoodProperties> {
    let name = item_id_to_name(item_id)?;
    let (nutrition, sat_mod, eat_ticks, always_eat) = match name {
        "apple" => (4, 0.3, 32, false),
        "baked_potato" => (5, 0.6, 32, false),
        "beef" => (3, 0.3, 32, false),
        "bread" => (5, 0.6, 32, false),
        "carrot" => (3, 0.6, 32, false),
        "chicken" => (2, 0.3, 32, false),
        "cooked_beef" => (8, 0.8, 32, false),
        "cooked_chicken" => (6, 0.6, 32, false),
        "cooked_mutton" => (6, 0.8, 32, false),
        "cooked_porkchop" => (8, 0.8, 32, false),
        "cooked_rabbit" => (5, 0.6, 32, false),
        "cooked_salmon" => (6, 0.8, 32, false),
        "cooked_cod" => (5, 0.6, 32, false),
        "cookie" => (2, 0.1, 32, false),
        "dried_kelp" => (1, 0.3, 16, false),
        "enchanted_golden_apple" => (4, 1.2, 32, true),
        "golden_apple" => (4, 1.2, 32, true),
        "golden_carrot" => (6, 1.2, 32, false),
        "melon_slice" => (2, 0.3, 32, false),
        "mutton" => (2, 0.3, 32, false),
        "porkchop" => (3, 0.3, 32, false),
        "potato" => (1, 0.3, 32, false),
        "pumpkin_pie" => (8, 0.3, 32, false),
        "rabbit" => (3, 0.3, 32, false),
        "cod" => (2, 0.1, 32, false),
        "salmon" => (2, 0.1, 32, false),
        "sweet_berries" => (2, 0.1, 32, false),
        "glow_berries" => (2, 0.1, 32, false),
        _ => return None,
    };
    Some(FoodProperties {
        nutrition,
        saturation_modifier: sat_mod,
        eat_ticks,
        can_always_eat: always_eat,
    })
}

/// Returns the sound group name for a block (e.g., "stone", "grass", "wood").
/// Used to construct sound resource locations like "minecraft:block.stone.break".
pub fn block_sound_group(block_name: &str) -> &'static str {
    match block_name {
        // Stone-like blocks
        "stone" | "cobblestone" | "mossy_cobblestone" | "smooth_stone"
        | "stone_bricks" | "mossy_stone_bricks" | "cracked_stone_bricks"
        | "chiseled_stone_bricks" | "bricks" | "andesite" | "diorite" | "granite"
        | "polished_andesite" | "polished_diorite" | "polished_granite"
        | "deepslate" | "cobbled_deepslate" | "polished_deepslate" | "obsidian"
        | "furnace" | "lit_furnace" | "coal_ore" | "iron_ore" | "gold_ore"
        | "diamond_ore" | "emerald_ore" | "lapis_ore" | "redstone_ore"
        | "copper_ore" | "coal_block" | "iron_block" | "gold_block"
        | "diamond_block" | "emerald_block" | "lapis_block" | "redstone_block"
        | "copper_block" | "netherrack" | "end_stone" | "bedrock"
        | "stone_pressure_plate" | "stone_button" | "polished_blackstone_button"
        | "dispenser" | "dropper" | "observer" | "piston" | "sticky_piston"
        | "terracotta" | "prismarine" | "purpur_block" | "quartz_block"
        => "stone",

        // Dirt/gravel
        "dirt" | "coarse_dirt" | "rooted_dirt" | "farmland" | "dirt_path"
        | "clay" | "soul_sand" | "soul_soil" | "mycelium" | "podzol"
        => "gravel",

        // Grass
        "grass_block" | "moss_block" => "grass",

        // Sand
        "sand" | "red_sand" | "concrete_powder" => "sand",

        // Wood
        n if n.contains("planks") || n.contains("_log") || n.contains("_wood")
            || n.contains("_stem") || n.contains("_hyphae")
            || n == "crafting_table" || n == "chest" || n == "barrel"
            || n == "note_block" || n == "jukebox" || n == "bookshelf"
            || n.contains("_fence") && !n.contains("fence_gate")
            || n.contains("_slab") && (n.contains("oak") || n.contains("spruce")
                || n.contains("birch") || n.contains("jungle")
                || n.contains("acacia") || n.contains("dark_oak")
                || n.contains("mangrove") || n.contains("cherry")
                || n.contains("bamboo") || n.contains("crimson") || n.contains("warped"))
            || n.contains("_stairs") && (n.contains("oak") || n.contains("spruce")
                || n.contains("birch") || n.contains("jungle")
                || n.contains("acacia") || n.contains("dark_oak")
                || n.contains("mangrove") || n.contains("cherry")
                || n.contains("bamboo") || n.contains("crimson") || n.contains("warped"))
        => "wood",

        // Glass
        n if n.contains("glass") => "glass",

        // Wool/carpet
        n if n.contains("wool") || n.contains("carpet") => "wool",

        // Metal
        n if n.contains("iron_door") || n.contains("iron_trapdoor")
            || n.contains("iron_bars") || n.contains("chain")
            || n.contains("anvil") || n.contains("cauldron")
            || n.contains("hopper") || n.contains("lantern")
        => "metal",

        // Doors (wooden)
        n if n.contains("_door") && !n.contains("iron") => "wood",

        // Trapdoors (wooden)
        n if n.contains("_trapdoor") && !n.contains("iron") => "wood",

        // Fence gates
        n if n.contains("fence_gate") => "wood",

        // Levers
        "lever" => "stone",

        // Buttons (wooden)
        n if n.contains("_button") && !n.contains("stone") && !n.contains("polished") => "wood",

        // Torch
        n if n.contains("torch") => "wood",

        // Crop-like
        n if n.contains("crop") || n == "wheat" || n == "carrots"
            || n == "potatoes" || n == "beetroots" || n == "melon"
            || n == "pumpkin" || n == "sugar_cane" || n == "bamboo"
            || n.contains("leaves")
        => "grass",

        // Default to stone for anything unmatched
        _ => "stone",
    }
}

/// A shaped crafting recipe. Pattern is stored in a 3x3 grid (row-major), 0 means empty.
pub struct CraftingRecipe {
    pub pattern: [i32; 9],
    pub result_id: i32,
    pub result_count: i8,
    pub width: u8,
    pub height: u8,
}

/// Returns all crafting recipes.
pub fn crafting_recipes() -> &'static [CraftingRecipe] {
    use std::sync::LazyLock;
    static RECIPES: LazyLock<Vec<CraftingRecipe>> = LazyLock::new(build_recipes);
    &RECIPES
}

fn build_recipes() -> Vec<CraftingRecipe> {
    let id = |name: &str| -> i32 {
        item_name_to_id(name).unwrap_or_else(|| panic!("Unknown item: {}", name))
    };

    let mut recipes = Vec::new();
    let p = id("oak_planks");
    let s = id("stick");
    let c = id("cobblestone");

    // Planks from logs (4 planks per log)
    for log in &["oak_log", "spruce_log", "birch_log", "jungle_log", "acacia_log", "dark_oak_log"] {
        recipes.push(CraftingRecipe {
            pattern: [id(log), 0,0, 0,0,0, 0,0,0],
            result_id: id("oak_planks"), result_count: 4, width: 1, height: 1,
        });
    }

    // Sticks (4 sticks from 2 planks)
    recipes.push(CraftingRecipe {
        pattern: [p, 0,0, p, 0,0, 0,0,0],
        result_id: id("stick"), result_count: 4, width: 1, height: 2,
    });

    // Crafting table
    recipes.push(CraftingRecipe {
        pattern: [p, p, 0, p, p, 0, 0, 0, 0],
        result_id: id("crafting_table"), result_count: 1, width: 2, height: 2,
    });

    // Furnace
    recipes.push(CraftingRecipe {
        pattern: [c, c, c, c, 0, c, c, c, c],
        result_id: id("furnace"), result_count: 1, width: 3, height: 3,
    });

    // Chest
    recipes.push(CraftingRecipe {
        pattern: [p, p, p, p, 0, p, p, p, p],
        result_id: id("chest"), result_count: 1, width: 3, height: 3,
    });

    // Wooden pickaxe
    recipes.push(CraftingRecipe {
        pattern: [p, p, p, 0, s, 0, 0, s, 0],
        result_id: id("wooden_pickaxe"), result_count: 1, width: 3, height: 3,
    });

    // Wooden axe
    recipes.push(CraftingRecipe {
        pattern: [p, p, 0, p, s, 0, 0, s, 0],
        result_id: id("wooden_axe"), result_count: 1, width: 2, height: 3,
    });

    // Wooden shovel
    recipes.push(CraftingRecipe {
        pattern: [p, 0, 0, s, 0, 0, s, 0, 0],
        result_id: id("wooden_shovel"), result_count: 1, width: 1, height: 3,
    });

    // Wooden sword
    recipes.push(CraftingRecipe {
        pattern: [p, 0, 0, p, 0, 0, s, 0, 0],
        result_id: id("wooden_sword"), result_count: 1, width: 1, height: 3,
    });

    // Stone pickaxe
    recipes.push(CraftingRecipe {
        pattern: [c, c, c, 0, s, 0, 0, s, 0],
        result_id: id("stone_pickaxe"), result_count: 1, width: 3, height: 3,
    });

    // Stone axe
    recipes.push(CraftingRecipe {
        pattern: [c, c, 0, c, s, 0, 0, s, 0],
        result_id: id("stone_axe"), result_count: 1, width: 2, height: 3,
    });

    // Stone shovel
    recipes.push(CraftingRecipe {
        pattern: [c, 0, 0, s, 0, 0, s, 0, 0],
        result_id: id("stone_shovel"), result_count: 1, width: 1, height: 3,
    });

    // Stone sword
    recipes.push(CraftingRecipe {
        pattern: [c, 0, 0, c, 0, 0, s, 0, 0],
        result_id: id("stone_sword"), result_count: 1, width: 1, height: 3,
    });

    // Torches (4 from coal + stick)
    recipes.push(CraftingRecipe {
        pattern: [id("coal"), 0,0, s, 0,0, 0,0,0],
        result_id: id("torch"), result_count: 4, width: 1, height: 2,
    });

    // Iron tools
    let iron = id("iron_ingot");
    recipes.push(CraftingRecipe {
        pattern: [iron, iron, iron, 0, s, 0, 0, s, 0],
        result_id: id("iron_pickaxe"), result_count: 1, width: 3, height: 3,
    });
    recipes.push(CraftingRecipe {
        pattern: [iron, iron, 0, iron, s, 0, 0, s, 0],
        result_id: id("iron_axe"), result_count: 1, width: 2, height: 3,
    });
    recipes.push(CraftingRecipe {
        pattern: [iron, 0, 0, s, 0, 0, s, 0, 0],
        result_id: id("iron_shovel"), result_count: 1, width: 1, height: 3,
    });
    recipes.push(CraftingRecipe {
        pattern: [iron, 0, 0, iron, 0, 0, s, 0, 0],
        result_id: id("iron_sword"), result_count: 1, width: 1, height: 3,
    });

    // Diamond tools
    let dia = id("diamond");
    recipes.push(CraftingRecipe {
        pattern: [dia, dia, dia, 0, s, 0, 0, s, 0],
        result_id: id("diamond_pickaxe"), result_count: 1, width: 3, height: 3,
    });
    recipes.push(CraftingRecipe {
        pattern: [dia, dia, 0, dia, s, 0, 0, s, 0],
        result_id: id("diamond_axe"), result_count: 1, width: 2, height: 3,
    });
    recipes.push(CraftingRecipe {
        pattern: [dia, 0, 0, s, 0, 0, s, 0, 0],
        result_id: id("diamond_shovel"), result_count: 1, width: 1, height: 3,
    });
    recipes.push(CraftingRecipe {
        pattern: [dia, 0, 0, dia, 0, 0, s, 0, 0],
        result_id: id("diamond_sword"), result_count: 1, width: 1, height: 3,
    });

    // Armor recipes: helmet (XXX, X.X), chestplate (X.X, XXX, XXX), leggings (XXX, X.X, X.X), boots (X.X, X.X)
    for (material, prefix) in [
        (id("leather"), "leather"),
        (id("iron_ingot"), "iron"),
        (id("gold_ingot"), "golden"),
        (id("diamond"), "diamond"),
    ] {
        let m = material;
        // Helmet
        recipes.push(CraftingRecipe {
            pattern: [m, m, m, m, 0, m, 0, 0, 0],
            result_id: id(&format!("{}_helmet", prefix)), result_count: 1, width: 3, height: 2,
        });
        // Chestplate
        recipes.push(CraftingRecipe {
            pattern: [m, 0, m, m, m, m, m, m, m],
            result_id: id(&format!("{}_chestplate", prefix)), result_count: 1, width: 3, height: 3,
        });
        // Leggings
        recipes.push(CraftingRecipe {
            pattern: [m, m, m, m, 0, m, m, 0, m],
            result_id: id(&format!("{}_leggings", prefix)), result_count: 1, width: 3, height: 3,
        });
        // Boots
        recipes.push(CraftingRecipe {
            pattern: [m, 0, m, m, 0, m, 0, 0, 0],
            result_id: id(&format!("{}_boots", prefix)), result_count: 1, width: 3, height: 2,
        });
    }

    // Beds: 3 wool + 3 planks
    for (wool, bed) in [
        ("white_wool", "white_bed"),
        ("orange_wool", "orange_bed"),
        ("magenta_wool", "magenta_bed"),
        ("light_blue_wool", "light_blue_bed"),
        ("yellow_wool", "yellow_bed"),
        ("lime_wool", "lime_bed"),
        ("pink_wool", "pink_bed"),
        ("gray_wool", "gray_bed"),
        ("light_gray_wool", "light_gray_bed"),
        ("cyan_wool", "cyan_bed"),
        ("purple_wool", "purple_bed"),
        ("blue_wool", "blue_bed"),
        ("brown_wool", "brown_bed"),
        ("green_wool", "green_bed"),
        ("red_wool", "red_bed"),
        ("black_wool", "black_bed"),
    ] {
        let w = id(wool);
        recipes.push(CraftingRecipe {
            pattern: [w, w, w, p, p, p, 0, 0, 0],
            result_id: id(bed), result_count: 1, width: 3, height: 2,
        });
    }

    recipes
}

/// Returns (defense_points, armor_toughness) for armor items.
/// Defense points are the armor icons shown on the HUD.
pub fn armor_defense(item_name: &str) -> Option<(i32, f32)> {
    match item_name {
        // Leather: helmet=1, chest=3, legs=2, boots=1, toughness=0
        "leather_helmet" => Some((1, 0.0)),
        "leather_chestplate" => Some((3, 0.0)),
        "leather_leggings" => Some((2, 0.0)),
        "leather_boots" => Some((1, 0.0)),
        // Chainmail
        "chainmail_helmet" => Some((2, 0.0)),
        "chainmail_chestplate" => Some((5, 0.0)),
        "chainmail_leggings" => Some((4, 0.0)),
        "chainmail_boots" => Some((1, 0.0)),
        // Iron
        "iron_helmet" => Some((2, 0.0)),
        "iron_chestplate" => Some((6, 0.0)),
        "iron_leggings" => Some((5, 0.0)),
        "iron_boots" => Some((2, 0.0)),
        // Gold
        "golden_helmet" => Some((2, 0.0)),
        "golden_chestplate" => Some((5, 0.0)),
        "golden_leggings" => Some((3, 0.0)),
        "golden_boots" => Some((1, 0.0)),
        // Diamond
        "diamond_helmet" => Some((3, 2.0)),
        "diamond_chestplate" => Some((8, 2.0)),
        "diamond_leggings" => Some((6, 2.0)),
        "diamond_boots" => Some((3, 2.0)),
        // Netherite
        "netherite_helmet" => Some((3, 3.0)),
        "netherite_chestplate" => Some((8, 3.0)),
        "netherite_leggings" => Some((6, 3.0)),
        "netherite_boots" => Some((3, 3.0)),
        _ => None,
    }
}

/// Returns the equipment slot index for armor items.
/// Slot IDs: 2=boots(FEET), 3=leggings(LEGS), 4=chest(CHEST), 5=head(HELMET)
/// Returns None if not an armor item.
pub fn armor_equipment_slot(item_name: &str) -> Option<u8> {
    if item_name.contains("helmet") { Some(5) }
    else if item_name.contains("chestplate") { Some(4) }
    else if item_name.contains("leggings") { Some(3) }
    else if item_name.contains("boots") { Some(2) }
    else { None }
}

/// Returns the inventory slot index for armor items (5=helmet, 6=chest, 7=legs, 8=boots).
pub fn armor_inventory_slot(item_name: &str) -> Option<usize> {
    if item_name.contains("helmet") { Some(5) }
    else if item_name.contains("chestplate") { Some(6) }
    else if item_name.contains("leggings") { Some(7) }
    else if item_name.contains("boots") { Some(8) }
    else { None }
}

/// Returns max durability for tools and armor, or 0 if not damageable.
pub fn item_max_durability(item_name: &str) -> i32 {
    match item_name {
        // Swords
        "wooden_sword" => 59,
        "stone_sword" => 131,
        "iron_sword" => 250,
        "golden_sword" => 32,
        "diamond_sword" => 1561,
        "netherite_sword" => 2031,
        // Pickaxes
        "wooden_pickaxe" => 59,
        "stone_pickaxe" => 131,
        "iron_pickaxe" => 250,
        "golden_pickaxe" => 32,
        "diamond_pickaxe" => 1561,
        "netherite_pickaxe" => 2031,
        // Axes
        "wooden_axe" => 59,
        "stone_axe" => 131,
        "iron_axe" => 250,
        "golden_axe" => 32,
        "diamond_axe" => 1561,
        "netherite_axe" => 2031,
        // Shovels
        "wooden_shovel" => 59,
        "stone_shovel" => 131,
        "iron_shovel" => 250,
        "golden_shovel" => 32,
        "diamond_shovel" => 1561,
        "netherite_shovel" => 2031,
        // Hoes
        "wooden_hoe" => 59,
        "stone_hoe" => 131,
        "iron_hoe" => 250,
        "golden_hoe" => 32,
        "diamond_hoe" => 1561,
        "netherite_hoe" => 2031,
        // Armor: durability = multiplier * base
        // Leather (base 5): helmet=11*5=55, chest=16*5=80, legs=15*5=75, boots=13*5=65
        "leather_helmet" => 55,
        "leather_chestplate" => 80,
        "leather_leggings" => 75,
        "leather_boots" => 65,
        // Chainmail (base 15): 165, 240, 225, 195
        "chainmail_helmet" => 165,
        "chainmail_chestplate" => 240,
        "chainmail_leggings" => 225,
        "chainmail_boots" => 195,
        // Iron (base 15): 165, 240, 225, 195
        "iron_helmet" => 165,
        "iron_chestplate" => 240,
        "iron_leggings" => 225,
        "iron_boots" => 195,
        // Gold (base 7): 77, 112, 105, 91
        "golden_helmet" => 77,
        "golden_chestplate" => 112,
        "golden_leggings" => 105,
        "golden_boots" => 91,
        // Diamond (base 33): 363, 528, 495, 429
        "diamond_helmet" => 363,
        "diamond_chestplate" => 528,
        "diamond_leggings" => 495,
        "diamond_boots" => 429,
        // Netherite (base 37): 407, 592, 555, 481
        "netherite_helmet" => 407,
        "netherite_chestplate" => 592,
        "netherite_leggings" => 555,
        "netherite_boots" => 481,
        // Other damageable items
        "bow" => 384,
        "crossbow" => 465,
        "trident" => 250,
        "shield" => 336,
        "flint_and_steel" => 64,
        "shears" => 238,
        "fishing_rod" => 64,
        _ => 0,
    }
}

/// Returns the attack damage bonus for weapons/tools.
/// This is the bonus damage added on top of the base 1.0 damage.
pub fn item_attack_damage(item_name: &str) -> f32 {
    match item_name {
        // Swords: base damage = 4/5/6/7/4
        "wooden_sword" => 4.0,
        "stone_sword" => 5.0,
        "iron_sword" => 6.0,
        "diamond_sword" => 7.0,
        "netherite_sword" => 8.0,
        "golden_sword" => 4.0,
        // Axes: 7/9/9/9/5
        "wooden_axe" => 7.0,
        "stone_axe" => 9.0,
        "iron_axe" => 9.0,
        "diamond_axe" => 9.0,
        "netherite_axe" => 10.0,
        "golden_axe" => 7.0,
        // Pickaxes
        "wooden_pickaxe" => 2.0,
        "stone_pickaxe" => 3.0,
        "iron_pickaxe" => 4.0,
        "diamond_pickaxe" => 5.0,
        "netherite_pickaxe" => 6.0,
        "golden_pickaxe" => 2.0,
        // Shovels
        "wooden_shovel" => 2.5,
        "stone_shovel" => 3.5,
        "iron_shovel" => 4.5,
        "diamond_shovel" => 5.5,
        "netherite_shovel" => 6.5,
        "golden_shovel" => 2.5,
        _ => 1.0,
    }
}

// Bed block state IDs: 16 states per color, 16 bed colors (white through black).
// State = min + facing*4 + occupied*2 + part
// facing: north=0, south=1, west=2, east=3
// occupied: false=0, true=1
// part: head=0, foot=1
const BED_MIN_STATE: i32 = 1688; // white_bed min
const BED_MAX_STATE: i32 = 1943; // black_bed max

/// Returns true if the given block state is a bed block.
pub fn is_bed(state_id: i32) -> bool {
    (BED_MIN_STATE..=BED_MAX_STATE).contains(&state_id)
}

/// Returns the facing direction index (0=north, 1=south, 2=west, 3=east) for a bed state.
pub fn bed_facing(state_id: i32) -> i32 {
    if !is_bed(state_id) { return 0; }
    let rel = (state_id - BED_MIN_STATE) % 16;
    rel / 4
}

/// Returns true if this bed state is the head part (not the foot).
pub fn bed_is_head(state_id: i32) -> bool {
    if !is_bed(state_id) { return false; }
    let rel = (state_id - BED_MIN_STATE) % 16;
    (rel % 2) == 0 // part: head=0, foot=1
}

/// Returns the offset from foot to head for a bed facing direction.
/// facing: north=0 → (0,0,-1), south=1 → (0,0,1), west=2 → (-1,0,0), east=3 → (1,0,0)
/// Wait — in MC, beds: the HEAD is in the direction the player faces WHEN LYING DOWN.
/// foot → head: north → south (z+1), south → north (z-1), west → east (x+1), east → west (x-1)
/// Actually in MC: facing is direction the head faces away from the foot.
/// A north-facing bed: foot is at z, head is at z-1 (the head faces north).
/// No wait — checking BedBlock.java: headPos = pos.relative(state.getValue(FACING))
/// So for facing=north: head = foot + north = foot + (0,0,-1)
pub fn bed_head_offset(facing: i32) -> (i32, i32) {
    match facing {
        0 => (0, -1),  // north: dz=-1
        1 => (0, 1),   // south: dz=+1
        2 => (-1, 0),  // west: dx=-1
        3 => (1, 0),   // east: dx=+1
        _ => (0, 0),
    }
}

/// Returns the facing index for a given yaw angle (player's look direction).
/// Used when placing beds to determine facing.
pub fn yaw_to_facing(yaw: f32) -> i32 {
    // MC facing: south=0, west=1, north=2, east=3 in some contexts
    // But bed facing: north=0, south=1, west=2, east=3
    let angle = ((yaw % 360.0) + 360.0) % 360.0;
    if angle >= 315.0 || angle < 45.0 { 1 }   // south (yaw 0 = looking south)
    else if angle < 135.0 { 2 }                 // west
    else if angle < 225.0 { 0 }                 // north
    else { 3 }                                   // east
}

/// Compute bed block state for a given bed color's min state, facing, occupied, and part.
pub fn bed_state(min_state: i32, facing: i32, occupied: bool, is_head: bool) -> i32 {
    min_state + facing * 4 + (occupied as i32) * 2 + (!is_head as i32)
}

/// Returns the min state ID for a bed item (by item name), or None.
pub fn bed_min_state_for_item(item_name: &str) -> Option<i32> {
    match item_name {
        "white_bed" => Some(1688),
        "orange_bed" => Some(1704),
        "magenta_bed" => Some(1720),
        "light_blue_bed" => Some(1736),
        "yellow_bed" => Some(1752),
        "lime_bed" => Some(1768),
        "pink_bed" => Some(1784),
        "gray_bed" => Some(1800),
        "light_gray_bed" => Some(1816),
        "cyan_bed" => Some(1832),
        "purple_bed" => Some(1848),
        "blue_bed" => Some(1864),
        "brown_bed" => Some(1880),
        "green_bed" => Some(1896),
        "red_bed" => Some(1912),
        "black_bed" => Some(1928),
        _ => None,
    }
}

/// Toggle the occupied state of a bed block state.
pub fn bed_set_occupied(state_id: i32, occupied: bool) -> i32 {
    if !is_bed(state_id) { return state_id; }
    let rel = (state_id - BED_MIN_STATE) % 16;
    let base = state_id - rel;
    let facing = rel / 4;
    let part = rel % 2;
    base + facing * 4 + (occupied as i32) * 2 + part
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_lookups() {
        assert_eq!(block_name_to_default_state("air"), Some(0));
        assert_eq!(block_name_to_default_state("stone"), Some(1));
        assert_eq!(block_name_to_default_state("grass_block"), Some(9));
        assert_eq!(block_name_to_default_state("bedrock"), Some(79));
        assert_eq!(block_name_to_default_state("nonexistent"), None);
    }

    #[test]
    fn test_item_lookups() {
        assert_eq!(item_name_to_id("stone"), Some(1));
        assert_eq!(item_name_to_id("air"), Some(0));
        assert!(item_name_to_id("nonexistent").is_none());
    }

    #[test]
    fn test_item_to_block() {
        let stone_item = item_name_to_id("stone").unwrap();
        assert_eq!(item_id_to_block_state(stone_item), Some(1));
        let dirt_item = item_name_to_id("dirt").unwrap();
        assert_eq!(item_id_to_block_state(dirt_item), Some(10));
    }

    #[test]
    fn test_block_state_to_item() {
        let stone_item = block_state_to_item_id(1);
        assert!(stone_item.is_some());
    }

    #[test]
    fn test_item_id_to_name() {
        assert_eq!(item_id_to_name(1), Some("stone"));
        assert_eq!(item_id_to_name(0), Some("air"));
    }

    #[test]
    fn test_stack_size() {
        assert_eq!(item_id_to_stack_size(1), Some(64));
    }

    #[test]
    fn test_block_hardness() {
        assert_eq!(block_state_to_hardness(1), Some((1.5, true))); // stone
        assert_eq!(block_state_to_hardness(79), Some((-1.0, false))); // bedrock
        assert_eq!(block_state_to_hardness(0), Some((0.0, false))); // air
        assert_eq!(block_state_to_hardness(10), Some((0.5, true))); // dirt
    }

    #[test]
    fn test_block_drops() {
        assert_eq!(block_state_to_drops(1), &[35]); // stone -> cobblestone
        assert_eq!(block_state_to_drops(10), &[28]); // dirt -> dirt
        assert!(block_state_to_drops(0).is_empty()); // air -> nothing
    }

    #[test]
    fn test_harvest_tools() {
        let tools = block_state_to_harvest_tools(1).unwrap(); // stone requires pickaxes
        assert!(tools.contains(&820)); // wooden_pickaxe
        assert!(tools.contains(&845)); // netherite_pickaxe
        assert_eq!(block_state_to_harvest_tools(10), None); // dirt needs no tool
    }

    #[test]
    fn test_crafting_recipes() {
        let recipes = crafting_recipes();
        assert!(!recipes.is_empty());
        // Verify sticks recipe exists
        let plank_id = item_name_to_id("oak_planks").unwrap();
        let stick_id = item_name_to_id("stick").unwrap();
        let stick_recipe = recipes.iter().find(|r| r.result_id == stick_id);
        assert!(stick_recipe.is_some());
        let r = stick_recipe.unwrap();
        assert_eq!(r.result_count, 4);
        assert_eq!(r.pattern[0], plank_id);
        assert_eq!(r.pattern[3], plank_id);
    }

    #[test]
    fn test_fuel_and_smelting() {
        let coal_id = item_name_to_id("coal").unwrap();
        assert_eq!(fuel_burn_time(coal_id), Some(1600));
        let cobble_id = item_name_to_id("cobblestone").unwrap();
        let stone_id = item_name_to_id("stone").unwrap();
        assert_eq!(smelting_result(cobble_id), Some((stone_id, 200)));
    }

    #[test]
    fn test_interactive_blocks() {
        // Lever: 5626 is floor/north/powered=false
        // Toggle should give 5627 (powered=true), toggle back gives 5626
        let toggled = toggle_interactive_block(5626).unwrap();
        assert_eq!(toggled, 5627);
        assert_eq!(toggle_interactive_block(5627).unwrap(), 5626);

        // Lever default 5635: wall/north/powered=true, toggle gives 5634 (powered=false)
        assert_eq!(toggle_interactive_block(5635).unwrap(), 5634);

        // Oak door: 4590 (north, upper, left, open=false, powered=false)
        // open has stride 2, so toggle open: 4590 + 2 = 4592
        let toggled = toggle_interactive_block(4590).unwrap();
        assert_eq!(toggled, 4592);
        assert_eq!(toggle_interactive_block(4592).unwrap(), 4590);

        // Door other half offset
        let offset = door_other_half_offset(4601).unwrap();
        // 4601: rel=11, bit=(11/8)%2=1 (lower), so offset=-8
        assert_eq!(offset, -8);

        // Stone is not interactive
        assert!(toggle_interactive_block(1).is_none());

        // Button reset ticks
        assert_eq!(button_reset_ticks(5748), Some(20)); // stone_button
        assert_eq!(button_reset_ticks(8611), Some(30)); // oak_button
        assert!(button_reset_ticks(1).is_none()); // stone
    }

    #[test]
    fn test_food_properties() {
        let bread_id = item_name_to_id("bread").unwrap();
        let props = food_properties(bread_id).unwrap();
        assert_eq!(props.nutrition, 5);
        assert!((props.saturation_modifier - 0.6).abs() < 0.01);
        assert_eq!(props.eat_ticks, 32);
        assert!(!props.can_always_eat);

        let golden_apple_id = item_name_to_id("golden_apple").unwrap();
        let props = food_properties(golden_apple_id).unwrap();
        assert!(props.can_always_eat);

        // Non-food item
        let stone_id = item_name_to_id("stone").unwrap();
        assert!(food_properties(stone_id).is_none());

        // Meat smelting
        let beef_id = item_name_to_id("beef").unwrap();
        let cooked_beef_id = item_name_to_id("cooked_beef").unwrap();
        assert_eq!(smelting_result(beef_id), Some((cooked_beef_id, 200)));
    }

    #[test]
    fn test_armor_and_durability() {
        // Armor defense values
        assert_eq!(armor_defense("diamond_chestplate"), Some((8, 2.0)));
        assert_eq!(armor_defense("iron_helmet"), Some((2, 0.0)));
        assert_eq!(armor_defense("netherite_boots"), Some((3, 3.0)));
        assert_eq!(armor_defense("stone"), None);

        // Armor slots
        assert_eq!(armor_equipment_slot("diamond_helmet"), Some(5));
        assert_eq!(armor_equipment_slot("iron_chestplate"), Some(4));
        assert_eq!(armor_equipment_slot("golden_leggings"), Some(3));
        assert_eq!(armor_equipment_slot("leather_boots"), Some(2));
        assert_eq!(armor_equipment_slot("stone"), None);

        // Inventory slots
        assert_eq!(armor_inventory_slot("diamond_helmet"), Some(5));
        assert_eq!(armor_inventory_slot("iron_boots"), Some(8));

        // Tool durability
        assert_eq!(item_max_durability("diamond_pickaxe"), 1561);
        assert_eq!(item_max_durability("wooden_sword"), 59);
        assert_eq!(item_max_durability("netherite_axe"), 2031);
        assert_eq!(item_max_durability("stone"), 0);

        // Armor durability
        assert_eq!(item_max_durability("diamond_helmet"), 363);
        assert_eq!(item_max_durability("iron_chestplate"), 240);

        // Attack damage
        assert_eq!(item_attack_damage("diamond_sword"), 7.0);
        assert_eq!(item_attack_damage("netherite_axe"), 10.0);
        assert_eq!(item_attack_damage("stone"), 1.0);
    }

    #[test]
    fn test_bed_data() {
        // All bed states are in range 1688..=1943
        assert!(is_bed(1688)); // white bed foot
        assert!(is_bed(1943)); // black bed head
        assert!(!is_bed(1687));
        assert!(!is_bed(1944));
        assert!(!is_bed(0)); // air

        // Bed state encoding: min + facing*4 + occupied*2 + part
        // White bed (min=1688), north facing (0), not occupied
        assert_eq!(bed_state(1688, 0, false, false), 1689); // foot = +1 (is_head=false → +1)
        assert_eq!(bed_state(1688, 0, false, true), 1688);  // head = +0

        // bed_is_head: state_offset % 2 == 0 means head (part=0=head, part=1=foot)
        assert!(bed_is_head(1688)); // head
        assert!(!bed_is_head(1689)); // foot

        // Facing extraction: north=0, south=1, west=2, east=3
        assert_eq!(bed_facing(1688), 0); // north
        assert_eq!(bed_facing(1692), 1); // south
        assert_eq!(bed_facing(1696), 2); // west
        assert_eq!(bed_facing(1700), 3); // east

        // Head offset by facing (foot → head direction)
        assert_eq!(bed_head_offset(0), (0, -1));  // north: -Z
        assert_eq!(bed_head_offset(1), (0, 1));   // south: +Z
        assert_eq!(bed_head_offset(2), (-1, 0));  // west: -X
        assert_eq!(bed_head_offset(3), (1, 0));   // east: +X

        // Yaw to facing: yaw 0 = south in MC
        assert_eq!(yaw_to_facing(0.0), 1);    // south
        assert_eq!(yaw_to_facing(90.0), 2);   // west
        assert_eq!(yaw_to_facing(180.0), 0);  // north
        assert_eq!(yaw_to_facing(-90.0), 3);  // east

        // Set occupied
        let head_unoccupied = bed_state(1688, 0, false, true); // 1688
        let head_occupied = bed_set_occupied(head_unoccupied, true);
        assert_eq!(head_occupied, head_unoccupied + 2);
        assert_eq!(bed_set_occupied(head_occupied, false), head_unoccupied);

        // Bed crafting recipes exist (white bed = 3 white_wool + 3 planks)
        let white_bed_id = item_name_to_id("white_bed").unwrap();
        let recipe = crafting_recipes().iter().find(|r| r.result_id == white_bed_id);
        assert!(recipe.is_some(), "White bed crafting recipe should exist");
        assert_eq!(recipe.unwrap().result_count, 1);
    }
}
