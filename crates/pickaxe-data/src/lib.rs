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

    // Bow: string + sticks
    let string_id = id("string");
    recipes.push(CraftingRecipe {
        pattern: [0, s, string_id, s, 0, string_id, 0, s, string_id],
        result_id: id("bow"), result_count: 1, width: 3, height: 3,
    });

    // Arrow: flint + stick + feather
    let flint = id("flint");
    let feather = id("feather");
    recipes.push(CraftingRecipe {
        pattern: [flint, 0, 0, s, 0, 0, feather, 0, 0],
        result_id: id("arrow"), result_count: 4, width: 1, height: 3,
    });

    // Shield: iron ingot + planks
    let iron = id("iron_ingot");
    recipes.push(CraftingRecipe {
        pattern: [p, iron, p, p, p, p, 0, p, 0],
        result_id: id("shield"), result_count: 1, width: 3, height: 3,
    });

    // Fishing rod: sticks + string
    let string_id2 = id("string");
    recipes.push(CraftingRecipe {
        pattern: [0, 0, s, 0, s, string_id2, s, 0, string_id2],
        result_id: id("fishing_rod"), result_count: 1, width: 3, height: 3,
    });

    // Hoes (material + stick pattern, like swords but horizontal top)
    recipes.push(CraftingRecipe {
        pattern: [p, p, 0, 0, s, 0, 0, s, 0],
        result_id: id("wooden_hoe"), result_count: 1, width: 2, height: 3,
    });
    recipes.push(CraftingRecipe {
        pattern: [c, c, 0, 0, s, 0, 0, s, 0],
        result_id: id("stone_hoe"), result_count: 1, width: 2, height: 3,
    });
    let iron = id("iron_ingot");
    recipes.push(CraftingRecipe {
        pattern: [iron, iron, 0, 0, s, 0, 0, s, 0],
        result_id: id("iron_hoe"), result_count: 1, width: 2, height: 3,
    });
    let dia = id("diamond");
    recipes.push(CraftingRecipe {
        pattern: [dia, dia, 0, 0, s, 0, 0, s, 0],
        result_id: id("diamond_hoe"), result_count: 1, width: 2, height: 3,
    });

    // Bread: 3 wheat in a row
    let wheat = id("wheat");
    recipes.push(CraftingRecipe {
        pattern: [wheat, wheat, wheat, 0, 0, 0, 0, 0, 0],
        result_id: id("bread"), result_count: 1, width: 3, height: 1,
    });

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

/// Returns true if the given item name is an axe (can disable shields).
pub fn is_axe(item_name: &str) -> bool {
    matches!(item_name, "wooden_axe" | "stone_axe" | "iron_axe" | "golden_axe" | "diamond_axe" | "netherite_axe")
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

// === Fluid Data ===

/// Water source block state ID (level=0).
pub const WATER_SOURCE: i32 = 80;
/// Lava source block state ID (level=0).
pub const LAVA_SOURCE: i32 = 96;

/// Check if a block state is any water block (source or flowing, state IDs 80-95).
pub fn is_water(state_id: i32) -> bool {
    (80..=95).contains(&state_id)
}

/// Check if a block state is any lava block (source or flowing, state IDs 96-111).
pub fn is_lava(state_id: i32) -> bool {
    (96..=111).contains(&state_id)
}

/// Get the water level (0=source/full, 1-7=flowing height, 8-15=falling).
/// Returns None if not a water block.
pub fn water_level(state_id: i32) -> Option<i32> {
    if is_water(state_id) {
        Some(state_id - 80)
    } else {
        None
    }
}

/// Get the fluid height as a fraction of a block (0.0-1.0).
/// Source blocks (level 0) have height 8/9 ≈ 0.889.
/// Flowing blocks have height = (8 - level) / 9.
/// Falling blocks (level 8-15) have height 1.0.
pub fn fluid_height(state_id: i32) -> f64 {
    if is_water(state_id) {
        let level = state_id - 80;
        if level == 0 {
            8.0 / 9.0
        } else if level >= 8 {
            1.0 // falling water
        } else {
            (8 - level) as f64 / 9.0
        }
    } else if is_lava(state_id) {
        let level = state_id - 96;
        if level == 0 {
            8.0 / 9.0
        } else if level >= 8 {
            1.0 // falling lava
        } else {
            (8 - level) as f64 / 9.0
        }
    } else {
        0.0
    }
}

/// Get water block state for a given level (0-15).
/// Level 0 = source, 1-7 = flowing, 8+ = falling.
pub fn water_state_with_level(level: i32) -> i32 {
    80 + level.clamp(0, 15)
}

/// Get lava level (0=source/full, 1-7=flowing, 8-15=falling).
/// Returns None if not a lava block.
pub fn lava_level(state_id: i32) -> Option<i32> {
    if is_lava(state_id) {
        Some(state_id - 96)
    } else {
        None
    }
}

/// Get lava block state for a given level (0-15).
pub fn lava_state_with_level(level: i32) -> i32 {
    96 + level.clamp(0, 15)
}

/// Check if a block is any fluid (water or lava).
pub fn is_fluid(state_id: i32) -> bool {
    is_water(state_id) || is_lava(state_id)
}

/// Check if a fluid block is a source (level 0).
pub fn is_fluid_source(state_id: i32) -> bool {
    state_id == WATER_SOURCE || state_id == LAVA_SOURCE
}

/// Get the fluid amount (1-8) from a block state. Source = 8, level 1 = 7, etc.
/// Returns 0 if not a fluid.
pub fn fluid_amount(state_id: i32) -> i32 {
    let level = if is_water(state_id) {
        state_id - 80
    } else if is_lava(state_id) {
        state_id - 96
    } else {
        return 0;
    };
    if level == 0 { 8 } else if level >= 8 { 8 } else { 8 - level }
}

/// Check if a block is solid for fluid flow purposes (stops fluid from passing through).
/// Air, fluids, and non-solid blocks (flowers, torches, etc.) are NOT solid.
pub fn is_solid_for_fluid(name: &str) -> bool {
    match name {
        "" | "air" | "cave_air" | "void_air" => false,
        "water" | "lava" => false,
        // Non-solid blocks that water can flow through / destroy
        n if is_flower(n) => false,
        n if n.ends_with("_tulip") => false,
        "short_grass" | "tall_grass" | "fern" | "large_fern" | "dead_bush" => false,
        "dandelion" | "poppy" | "blue_orchid" | "allium" | "azure_bluet" => false,
        "oxeye_daisy" | "cornflower" | "lily_of_the_valley" | "wither_rose" => false,
        "sunflower" | "lilac" | "rose_bush" | "peony" | "torchflower" => false,
        "torch" | "wall_torch" | "soul_torch" | "soul_wall_torch" => false,
        "redstone_torch" | "redstone_wall_torch" => false,
        n if n.ends_with("_carpet") => false,
        "snow" => false,
        "sugar_cane" => false,
        "vine" | "glow_lichen" => false,
        "sweet_berry_bush" => false,
        "fire" | "soul_fire" => false,
        "redstone_wire" => false,
        "rail" | "powered_rail" | "detector_rail" | "activator_rail" => false,
        "tripwire" | "tripwire_hook" => false,
        _ => true,
    }
}

/// Check if a block should be destroyed (broken with drops) when water flows into it.
pub fn is_fluid_destructible(name: &str) -> bool {
    match name {
        n if is_flower(n) => true,
        n if n.ends_with("_tulip") => true,
        "short_grass" | "tall_grass" | "fern" | "large_fern" | "dead_bush" => true,
        "dandelion" | "poppy" | "blue_orchid" | "allium" | "azure_bluet" => true,
        "oxeye_daisy" | "cornflower" | "lily_of_the_valley" | "wither_rose" => true,
        "sunflower" | "lilac" | "rose_bush" | "peony" | "torchflower" => true,
        "torch" | "wall_torch" | "soul_torch" | "soul_wall_torch" => true,
        "redstone_torch" | "redstone_wall_torch" => true,
        n if n.ends_with("_carpet") => true,
        "snow" => true,
        "sugar_cane" => true,
        "vine" | "glow_lichen" => true,
        "sweet_berry_bush" => true,
        "redstone_wire" => true,
        "rail" | "powered_rail" | "detector_rail" | "activator_rail" => true,
        "tripwire" | "tripwire_hook" => true,
        _ => false,
    }
}

/// Check if a block name is a flower.
fn is_flower(name: &str) -> bool {
    matches!(name,
        "dandelion" | "poppy" | "blue_orchid" | "allium" | "azure_bluet"
        | "oxeye_daisy" | "cornflower" | "lily_of_the_valley" | "wither_rose"
        | "torchflower"
    ) || name.ends_with("_tulip")
}

// === Fire Block Data ===

/// Fire block state range: 2360-2871 (512 states).
/// Encoding: state = 2360 + age * 32 + east_bit * 16 + north_bit * 8 + south_bit * 4 + up_bit * 2 + west_bit * 1
/// where bit = 0 if face is true, 1 if face is false (MC encodes true before false).
const FIRE_MIN_STATE: i32 = 2360;
const FIRE_MAX_STATE: i32 = 2871;

/// Soul fire is a single state.
pub const SOUL_FIRE_STATE: i32 = 2872;

/// Check if a block state is fire or soul_fire.
pub fn is_fire(state_id: i32) -> bool {
    (state_id >= FIRE_MIN_STATE && state_id <= FIRE_MAX_STATE) || state_id == SOUL_FIRE_STATE
}

/// Get fire block state for age 0, no face connections.
pub fn fire_default_state() -> i32 {
    FIRE_MIN_STATE + 31 // age=0, all faces false = offset 31
}

/// Get fire block state for a given age (0-15), no face connections.
pub fn fire_state_with_age(age: i32) -> i32 {
    let age = age.clamp(0, 15);
    FIRE_MIN_STATE + age * 32 + 31 // all faces false
}

/// Extract age from a fire block state. Returns 0-15, or -1 if not fire.
pub fn fire_age(state_id: i32) -> i32 {
    if state_id >= FIRE_MIN_STATE && state_id <= FIRE_MAX_STATE {
        (state_id - FIRE_MIN_STATE) / 32
    } else {
        -1
    }
}

/// Get flammability data for a block: (ignite_odds, burn_odds).
/// ignite_odds = how easily fire can spread TO this block (0-60).
/// burn_odds = how quickly fire consumes this block (0-100).
/// Returns (0, 0) for non-flammable blocks.
pub fn block_flammability(name: &str) -> (i32, i32) {
    match name {
        // Wood products
        n if n.ends_with("_planks") || n.ends_with("_slab") => (5, 20),
        n if n.ends_with("_stairs") && (n.contains("oak") || n.contains("spruce") || n.contains("birch")
            || n.contains("jungle") || n.contains("acacia") || n.contains("dark_oak")
            || n.contains("mangrove") || n.contains("cherry") || n.contains("bamboo")
            || n.contains("crimson") || n.contains("warped")) => (5, 20),
        n if n.ends_with("_log") || n.ends_with("_wood") || n.ends_with("_stem") || n.ends_with("_hyphae") => (5, 5),
        n if n.ends_with("_fence") || n.ends_with("_fence_gate") => (5, 20),
        "bookshelf" | "chiseled_bookshelf" | "lectern" => (30, 20),
        // Leaves
        n if n.ends_with("_leaves") => (30, 60),
        // Wool and carpet
        n if n.ends_with("_wool") => (30, 60),
        n if n.ends_with("_carpet") => (60, 20),
        // Plants
        "short_grass" | "tall_grass" | "fern" | "large_fern" | "dead_bush" => (60, 100),
        n if n.ends_with("_tulip") || n == "dandelion" || n == "poppy" || n == "blue_orchid"
            || n == "allium" || n == "azure_bluet" || n == "oxeye_daisy" || n == "cornflower"
            || n == "lily_of_the_valley" || n == "sunflower" || n == "lilac" || n == "rose_bush"
            || n == "peony" || n == "wither_rose" || n == "torchflower" => (60, 100),
        "vine" | "glow_lichen" => (15, 100),
        "sweet_berry_bush" => (60, 100),
        // Other flammable
        "tnt" => (15, 100),
        "dried_kelp_block" => (30, 60),
        "hay_block" | "target" => (60, 20),
        "scaffolding" => (60, 60),
        "coal_block" => (5, 5),
        "bamboo" | "bamboo_block" | "stripped_bamboo_block" => (60, 60),
        "bee_nest" => (30, 20),
        "beehive" => (5, 20),
        _ => (0, 0),
    }
}

/// Check if a block name is flammable (can catch fire from adjacent fire).
pub fn is_flammable(name: &str) -> bool {
    let (ignite, _burn) = block_flammability(name);
    ignite > 0
}

// === Redstone Data ===

/// Redstone wire state range: 2978-4273 (1296 states).
/// Formula: state_id = 2978 + west*1 + south*3 + power*9 + north*144 + east*432
/// Connection values: up=0, side=1, none=2
const REDSTONE_WIRE_MIN: i32 = 2978;
const REDSTONE_WIRE_MAX: i32 = 4273;

/// Redstone torch: 5738=unlit, 5739=lit
const REDSTONE_TORCH_MIN: i32 = 5738;
/// Redstone wall torch: 5740-5747 (4 facing × 2 lit)
/// Formula: state_id = 5740 + lit*1 + facing*2
const REDSTONE_WALL_TORCH_MIN: i32 = 5740;

/// Repeater: 5881-5944 (64 states)
/// Formula: state_id = 5881 + powered*1 + locked*2 + facing*4 + (delay-1)*16
const REPEATER_MIN: i32 = 5881;
const REPEATER_MAX: i32 = 5944;

/// Redstone lamp: 7417=lit (true=0), 7418=unlit (false=1)
const REDSTONE_LAMP_LIT: i32 = 7417;
const REDSTONE_LAMP_UNLIT: i32 = 7418;

/// Check if a block state is redstone wire.
pub fn is_redstone_wire(state_id: i32) -> bool {
    (REDSTONE_WIRE_MIN..=REDSTONE_WIRE_MAX).contains(&state_id)
}

/// Get the power level (0-15) of a redstone wire block state.
pub fn redstone_wire_power(state_id: i32) -> Option<i32> {
    if !is_redstone_wire(state_id) { return None; }
    let offset = state_id - REDSTONE_WIRE_MIN;
    Some((offset / 9) % 16)
}

/// Build a redstone wire state with given power and all-none connections.
/// For simplicity, we use all-none connections (the client auto-renders connections).
pub fn redstone_wire_state(power: i32) -> i32 {
    // west=none(2), south=none(2), power, north=none(2), east=none(2)
    // = 2978 + 2*1 + 2*3 + power*9 + 2*144 + 2*432
    REDSTONE_WIRE_MIN + 2 + 6 + power.clamp(0, 15) * 9 + 288 + 864
}

/// Build a redstone wire state with specific connections.
/// Each connection: 0=up, 1=side, 2=none
pub fn redstone_wire_state_full(power: i32, north: i32, east: i32, south: i32, west: i32) -> i32 {
    REDSTONE_WIRE_MIN
        + west.clamp(0, 2)
        + south.clamp(0, 2) * 3
        + power.clamp(0, 15) * 9
        + north.clamp(0, 2) * 144
        + east.clamp(0, 2) * 432
}

/// Check if a block state is a redstone torch (standing or wall).
pub fn is_redstone_torch(state_id: i32) -> bool {
    state_id == 5738 || state_id == 5739 || (5740..=5747).contains(&state_id)
}

/// Check if a redstone torch is lit.
/// Standing: 5738=lit, 5739=unlit. Wall: even offset=lit, odd=unlit.
pub fn redstone_torch_is_lit(state_id: i32) -> bool {
    if state_id == 5738 || state_id == 5739 {
        state_id == 5738 // standing torch: 5738=lit, 5739=unlit
    } else if (5740..=5747).contains(&state_id) {
        // wall torch: lit = (state_id - 5740) % 2 == 0
        (state_id - REDSTONE_WALL_TORCH_MIN) % 2 == 0
    } else {
        false
    }
}

/// Set the lit state of a redstone torch. Returns new state_id.
pub fn redstone_torch_set_lit(state_id: i32, lit: bool) -> i32 {
    if state_id == 5738 || state_id == 5739 {
        if lit { 5738 } else { 5739 }
    } else if (5740..=5747).contains(&state_id) {
        let base = state_id - (state_id - REDSTONE_WALL_TORCH_MIN) % 2;
        if lit { base } else { base + 1 }
    } else {
        state_id
    }
}

/// Check if a block state is a repeater.
pub fn is_repeater(state_id: i32) -> bool {
    (REPEATER_MIN..=REPEATER_MAX).contains(&state_id)
}

/// Get repeater properties: (delay 1-4, facing 0-3, locked, powered).
/// Facing: north=0, south=1, west=2, east=3.
/// State layout: 5881 + delay_idx*16 + facing_idx*4 + locked_idx*2 + powered_idx
/// Bool index: true=0, false=1.
pub fn repeater_props(state_id: i32) -> Option<(i32, i32, bool, bool)> {
    if !is_repeater(state_id) { return None; }
    let offset = state_id - REPEATER_MIN;
    let powered_idx = offset % 2;
    let locked_idx = (offset / 2) % 2;
    let facing = (offset / 4) % 4;
    let delay = (offset / 16) + 1;
    Some((delay, facing, locked_idx == 0, powered_idx == 0))
}

/// Build a repeater state from properties.
/// Facing: north=0, south=1, west=2, east=3.
pub fn repeater_state(delay: i32, facing: i32, locked: bool, powered: bool) -> i32 {
    REPEATER_MIN
        + if powered { 0 } else { 1 }  // true=0, false=1
        + if locked { 0 } else { 2 }   // true=0, false=1
        + facing.clamp(0, 3) * 4
        + (delay.clamp(1, 4) - 1) * 16
}

/// Check if a block state is a redstone lamp.
pub fn is_redstone_lamp(state_id: i32) -> bool {
    state_id == REDSTONE_LAMP_LIT || state_id == REDSTONE_LAMP_UNLIT
}

/// Set redstone lamp lit state.
pub fn redstone_lamp_set_lit(lit: bool) -> i32 {
    if lit { REDSTONE_LAMP_LIT } else { REDSTONE_LAMP_UNLIT }
}

/// Check if a block state is any powered lever (powered=true).
/// Lever state layout: 5626 + face*8 + facing*2 + powered_idx (true=0, false=1).
pub fn is_lever_powered(state_id: i32) -> bool {
    if !(5626..=5649).contains(&state_id) { return false; }
    (state_id - 5626) % 2 == 0
}

/// Check if a block state is any powered button.
pub fn is_button_powered(state_id: i32) -> bool {
    let name = block_state_to_name(state_id).unwrap_or("");
    if !name.ends_with("_button") { return false; }
    // All buttons share same layout: powered = offset % 2 == 1
    // Stone button: 5748-5771, oak_button etc have similar layout
    // Use toggle_interactive_block to check
    // Simpler: just check if the "powered" variant exists
    // Button state layout: state = min + powered*1 + facing*2 + face*8
    // So powered = (state - min) % 2 == 1
    // We can check via the generic method
    // For now, check if toggling gives a lower state (powered→unpowered)
    if let Some(toggled) = toggle_interactive_block(state_id) {
        toggled < state_id // powered version is always +1 from unpowered
    } else {
        false
    }
}

/// Get the redstone power level output by a block (0 or 15 for most sources).
/// Returns 0 for non-powered blocks.
pub fn block_power_output(state_id: i32) -> i32 {
    // Powered lever
    if is_lever_powered(state_id) { return 15; }
    // Powered button
    if is_button_powered(state_id) { return 15; }
    // Lit redstone torch
    if is_redstone_torch(state_id) && redstone_torch_is_lit(state_id) { return 15; }
    // Redstone block (always outputs 15)
    if block_state_to_name(state_id) == Some("redstone_block") { return 15; }
    // Powered repeater
    if is_repeater(state_id) {
        if let Some((_, _, _, powered)) = repeater_props(state_id) {
            if powered { return 15; }
        }
    }
    0
}

/// Check if a block is a solid/opaque full block (redstone conductor).
/// Solid blocks transmit strong power and block wire connections.
pub fn is_solid_block(state_id: i32) -> bool {
    let name = match block_state_to_name(state_id) {
        Some(n) => n,
        None => return false,
    };
    matches!(name,
        "stone" | "granite" | "polished_granite" | "diorite" | "polished_diorite"
        | "andesite" | "polished_andesite" | "deepslate" | "cobbled_deepslate"
        | "polished_deepslate" | "calcite" | "tuff" | "dripstone_block"
        | "grass_block" | "dirt" | "coarse_dirt" | "podzol" | "cobblestone"
        | "oak_planks" | "spruce_planks" | "birch_planks" | "jungle_planks"
        | "acacia_planks" | "dark_oak_planks" | "mangrove_planks" | "cherry_planks"
        | "bamboo_planks" | "crimson_planks" | "warped_planks"
        | "bedrock" | "sand" | "red_sand" | "gravel" | "coal_ore" | "iron_ore"
        | "copper_ore" | "gold_ore" | "redstone_ore" | "lapis_ore" | "diamond_ore"
        | "emerald_ore" | "deepslate_coal_ore" | "deepslate_iron_ore"
        | "deepslate_copper_ore" | "deepslate_gold_ore" | "deepslate_redstone_ore"
        | "deepslate_lapis_ore" | "deepslate_diamond_ore" | "deepslate_emerald_ore"
        | "coal_block" | "iron_block" | "gold_block" | "diamond_block"
        | "emerald_block" | "lapis_block" | "netherite_block" | "copper_block"
        | "raw_iron_block" | "raw_copper_block" | "raw_gold_block"
        | "bricks" | "bookshelf" | "mossy_cobblestone" | "obsidian"
        | "end_stone" | "netherrack" | "soul_sand" | "soul_soil" | "basalt"
        | "smooth_basalt" | "polished_basalt" | "glowstone" | "clay"
        | "terracotta" | "packed_mud" | "mud_bricks" | "prismarine"
        | "prismarine_bricks" | "dark_prismarine" | "purpur_block"
        | "smooth_stone" | "sandstone" | "red_sandstone" | "cut_sandstone"
        | "cut_red_sandstone" | "smooth_sandstone" | "smooth_red_sandstone"
        | "quartz_block" | "smooth_quartz" | "nether_bricks" | "red_nether_bricks"
        | "stone_bricks" | "mossy_stone_bricks" | "cracked_stone_bricks"
        | "chiseled_stone_bricks" | "infested_stone" | "infested_cobblestone"
        | "infested_stone_bricks" | "infested_mossy_stone_bricks"
        | "infested_cracked_stone_bricks" | "infested_chiseled_stone_bricks"
        | "infested_deepslate" | "blackstone" | "polished_blackstone"
        | "polished_blackstone_bricks" | "cracked_polished_blackstone_bricks"
        | "chiseled_polished_blackstone" | "end_stone_bricks"
        | "redstone_lamp" | "hay_block" | "bone_block" | "dried_kelp_block"
        | "target" | "shroomlight" | "warped_wart_block" | "nether_wart_block"
        | "crying_obsidian" | "lodestone" | "tinted_glass"
        | "white_wool" | "orange_wool" | "magenta_wool" | "light_blue_wool"
        | "yellow_wool" | "lime_wool" | "pink_wool" | "gray_wool"
        | "light_gray_wool" | "cyan_wool" | "purple_wool" | "blue_wool"
        | "brown_wool" | "green_wool" | "red_wool" | "black_wool"
        | "white_concrete" | "orange_concrete" | "magenta_concrete" | "light_blue_concrete"
        | "yellow_concrete" | "lime_concrete" | "pink_concrete" | "gray_concrete"
        | "light_gray_concrete" | "cyan_concrete" | "purple_concrete" | "blue_concrete"
        | "brown_concrete" | "green_concrete" | "red_concrete" | "black_concrete"
        | "white_terracotta" | "orange_terracotta" | "magenta_terracotta"
        | "light_blue_terracotta" | "yellow_terracotta" | "lime_terracotta"
        | "pink_terracotta" | "gray_terracotta" | "light_gray_terracotta"
        | "cyan_terracotta" | "purple_terracotta" | "blue_terracotta"
        | "brown_terracotta" | "green_terracotta" | "red_terracotta" | "black_terracotta"
        | "white_glazed_terracotta" | "orange_glazed_terracotta"
        | "magenta_glazed_terracotta" | "light_blue_glazed_terracotta"
        | "yellow_glazed_terracotta" | "lime_glazed_terracotta"
        | "pink_glazed_terracotta" | "gray_glazed_terracotta"
        | "light_gray_glazed_terracotta" | "cyan_glazed_terracotta"
        | "purple_glazed_terracotta" | "blue_glazed_terracotta"
        | "brown_glazed_terracotta" | "green_glazed_terracotta"
        | "red_glazed_terracotta" | "black_glazed_terracotta"
        | "white_concrete_powder" | "orange_concrete_powder" | "magenta_concrete_powder"
        | "light_blue_concrete_powder" | "yellow_concrete_powder"
        | "lime_concrete_powder" | "pink_concrete_powder" | "gray_concrete_powder"
        | "light_gray_concrete_powder" | "cyan_concrete_powder"
        | "purple_concrete_powder" | "blue_concrete_powder"
        | "brown_concrete_powder" | "green_concrete_powder"
        | "red_concrete_powder" | "black_concrete_powder"
        | "amethyst_block" | "budding_amethyst" | "moss_block" | "muddy_mangrove_roots"
        | "sculk" | "sculk_catalyst" | "reinforced_deepslate"
    )
}

/// Get the facing direction a wall torch is pointing (0=north, 1=south, 2=west, 3=east).
/// Returns the direction the torch faces (away from the wall it's attached to).
pub fn redstone_wall_torch_facing(state_id: i32) -> Option<i32> {
    if !(REDSTONE_WALL_TORCH_MIN..=5747).contains(&state_id) { return None; }
    Some((state_id - REDSTONE_WALL_TORCH_MIN) / 2)
}

/// Get the facing direction of a repeater (0=north, 1=south, 2=west, 3=east).
/// This is the direction the repeater outputs to.
pub fn repeater_facing(state_id: i32) -> Option<i32> {
    repeater_props(state_id).map(|(_, facing, _, _)| facing)
}

/// Convert facing direction (0=north, 1=south, 2=west, 3=east) to dx, dz offset.
pub fn facing_to_offset(facing: i32) -> (i32, i32) {
    match facing {
        0 => (0, -1),  // north: -z
        1 => (0, 1),   // south: +z
        2 => (-1, 0),  // west: -x
        3 => (1, 0),   // east: +x
        _ => (0, 0),
    }
}

/// Get the opposite facing direction.
pub fn opposite_facing(facing: i32) -> i32 {
    match facing {
        0 => 1, // north → south
        1 => 0, // south → north
        2 => 3, // west → east
        3 => 2, // east → west
        _ => facing,
    }
}

// === Mob Data ===

/// Mob type constants (protocol entity type IDs for MC 1.21.1).
pub const ENTITY_TNT: i32 = 106;

pub const MOB_BAT: i32 = 6;
pub const MOB_CHICKEN: i32 = 19;
pub const MOB_COW: i32 = 22;
pub const MOB_CREEPER: i32 = 23;
pub const MOB_ENDERMAN: i32 = 33;
pub const MOB_PIG: i32 = 77;
pub const MOB_SHEEP: i32 = 87;
pub const MOB_SKELETON: i32 = 91;
pub const MOB_SLIME: i32 = 93;
pub const MOB_SPIDER: i32 = 100;
pub const MOB_ZOMBIE: i32 = 124;

/// Returns mob type name from entity type ID.
pub fn mob_type_name(type_id: i32) -> Option<&'static str> {
    match type_id {
        MOB_BAT => Some("bat"),
        MOB_CHICKEN => Some("chicken"),
        MOB_COW => Some("cow"),
        MOB_CREEPER => Some("creeper"),
        MOB_ENDERMAN => Some("enderman"),
        MOB_PIG => Some("pig"),
        MOB_SHEEP => Some("sheep"),
        MOB_SKELETON => Some("skeleton"),
        MOB_SLIME => Some("slime"),
        MOB_SPIDER => Some("spider"),
        MOB_ZOMBIE => Some("zombie"),
        _ => None,
    }
}

/// Reverse lookup: name → mob type ID.
pub fn mob_name_to_type(name: &str) -> Option<i32> {
    match name {
        "bat" => Some(MOB_BAT),
        "chicken" => Some(MOB_CHICKEN),
        "cow" => Some(MOB_COW),
        "creeper" => Some(MOB_CREEPER),
        "enderman" => Some(MOB_ENDERMAN),
        "pig" => Some(MOB_PIG),
        "sheep" => Some(MOB_SHEEP),
        "skeleton" => Some(MOB_SKELETON),
        "slime" => Some(MOB_SLIME),
        "spider" => Some(MOB_SPIDER),
        "zombie" => Some(MOB_ZOMBIE),
        _ => None,
    }
}

/// Returns the max health for a mob type.
pub fn mob_max_health(type_id: i32) -> f32 {
    match type_id {
        MOB_BAT => 6.0,
        MOB_CHICKEN => 4.0,
        MOB_COW => 10.0,
        MOB_CREEPER => 20.0,
        MOB_ENDERMAN => 40.0,
        MOB_PIG => 10.0,
        MOB_SHEEP => 8.0,
        MOB_SKELETON => 20.0,
        MOB_SLIME => 4.0,  // size 2 (default spawn)
        MOB_SPIDER => 16.0,
        MOB_ZOMBIE => 20.0,
        _ => 10.0,
    }
}

/// Returns the attack damage for a hostile mob type (0 for passive).
pub fn mob_attack_damage(type_id: i32) -> f32 {
    match type_id {
        MOB_CREEPER => 0.0,  // explosion damage, not melee
        MOB_ENDERMAN => 7.0,
        MOB_SKELETON => 2.0,  // bow damage, varies with difficulty
        MOB_SLIME => 2.0,     // size 2 damage
        MOB_SPIDER => 2.0,
        MOB_ZOMBIE => 3.0,
        _ => 0.0,
    }
}

/// Returns whether a mob type is hostile.
pub fn mob_is_hostile(type_id: i32) -> bool {
    matches!(type_id, MOB_CREEPER | MOB_ENDERMAN | MOB_SKELETON | MOB_SLIME | MOB_SPIDER | MOB_ZOMBIE)
}

/// Returns mob movement speed in blocks/tick.
pub fn mob_speed(type_id: i32) -> f64 {
    match type_id {
        MOB_BAT => 0.04,
        MOB_CHICKEN => 0.05,
        MOB_COW => 0.04,
        MOB_CREEPER => 0.05,
        MOB_ENDERMAN => 0.06,
        MOB_PIG => 0.05,
        MOB_SHEEP => 0.046,
        MOB_SKELETON => 0.05,
        MOB_SLIME => 0.04,
        MOB_SPIDER => 0.06,
        MOB_ZOMBIE => 0.046,
        _ => 0.04,
    }
}

/// Returns mob drops as a list of (item_name, min_count, max_count).
pub fn mob_drops(type_id: i32) -> &'static [(&'static str, i32, i32)] {
    match type_id {
        MOB_BAT => &[],
        MOB_CHICKEN => &[("chicken", 1, 1), ("feather", 0, 2)],
        MOB_COW => &[("beef", 1, 3), ("leather", 0, 2)],
        MOB_CREEPER => &[("gunpowder", 0, 2)],
        MOB_ENDERMAN => &[("ender_pearl", 0, 1)],
        MOB_PIG => &[("porkchop", 1, 3)],
        MOB_SHEEP => &[("mutton", 1, 2)],
        MOB_SKELETON => &[("arrow", 0, 2), ("bone", 0, 2)],
        MOB_SLIME => &[("slime_ball", 0, 2)],
        MOB_SPIDER => &[("string", 0, 2), ("spider_eye", 0, 1)],
        MOB_ZOMBIE => &[("rotten_flesh", 0, 2)],
        _ => &[],
    }
}

/// Returns XP dropped when this mob dies.
pub fn mob_xp_drop(type_id: i32) -> i32 {
    match type_id {
        MOB_BAT => 0,
        MOB_CHICKEN | MOB_COW | MOB_PIG | MOB_SHEEP => 3,
        MOB_CREEPER | MOB_ENDERMAN | MOB_SKELETON | MOB_SPIDER | MOB_ZOMBIE => 5,
        MOB_SLIME => 2,
        _ => 0,
    }
}

/// Returns the hitbox (width, height) for a mob type.
pub fn mob_hitbox(type_id: i32) -> (f64, f64) {
    match type_id {
        MOB_BAT => (0.5, 0.9),
        MOB_CHICKEN => (0.4, 0.7),
        MOB_COW => (0.9, 1.4),
        MOB_CREEPER => (0.6, 1.7),
        MOB_ENDERMAN => (0.6, 2.9),
        MOB_PIG => (0.9, 0.9),
        MOB_SHEEP => (0.9, 1.3),
        MOB_SKELETON => (0.6, 1.99),
        MOB_SLIME => (1.04, 1.04),  // size 2
        MOB_SPIDER => (1.4, 0.9),
        MOB_ZOMBIE => (0.6, 1.95),
        _ => (0.6, 1.8),
    }
}

/// Returns sound event names (ambient, hurt, death) for a mob type.
pub fn mob_sounds(type_id: i32) -> (&'static str, &'static str, &'static str) {
    match type_id {
        MOB_BAT => ("entity.bat.ambient", "entity.bat.hurt", "entity.bat.death"),
        MOB_CHICKEN => ("entity.chicken.ambient", "entity.chicken.hurt", "entity.chicken.death"),
        MOB_COW => ("entity.cow.ambient", "entity.cow.hurt", "entity.cow.death"),
        MOB_CREEPER => ("", "entity.creeper.hurt", "entity.creeper.death"),
        MOB_ENDERMAN => ("entity.enderman.ambient", "entity.enderman.hurt", "entity.enderman.death"),
        MOB_PIG => ("entity.pig.ambient", "entity.pig.hurt", "entity.pig.death"),
        MOB_SHEEP => ("entity.sheep.ambient", "entity.sheep.hurt", "entity.sheep.death"),
        MOB_SKELETON => ("entity.skeleton.ambient", "entity.skeleton.hurt", "entity.skeleton.death"),
        MOB_SLIME => ("", "entity.slime.hurt", "entity.slime.death"),
        MOB_SPIDER => ("entity.spider.ambient", "entity.spider.hurt", "entity.spider.death"),
        MOB_ZOMBIE => ("entity.zombie.ambient", "entity.zombie.hurt", "entity.zombie.death"),
        _ => ("", "", ""),
    }
}

/// Returns whether this mob type uses ranged attacks (skeletons).
pub fn mob_is_ranged(type_id: i32) -> bool {
    type_id == MOB_SKELETON
}

/// Returns whether this mob type explodes (creepers).
pub fn mob_is_explosive(type_id: i32) -> bool {
    type_id == MOB_CREEPER
}

/// Fishing loot: returns (item_name, count) based on a random value 0.0-1.0.
/// Loot distribution: 85% fish, 10% junk, 5% treasure.
/// Fish: cod 60%, salmon 25%, tropical_fish 2%, pufferfish 13%.
pub fn fishing_loot(roll: f64) -> (&'static str, i32) {
    if roll < 0.85 {
        // Fish category (remap roll to 0-1 within fish range)
        let fish_roll = roll / 0.85;
        if fish_roll < 0.60 { ("cod", 1) }
        else if fish_roll < 0.85 { ("salmon", 1) }
        else if fish_roll < 0.87 { ("tropical_fish", 1) }
        else { ("pufferfish", 1) }
    } else if roll < 0.95 {
        // Junk category (simplified)
        let junk_roll = (roll - 0.85) / 0.10;
        if junk_roll < 0.15 { ("leather", 1) }
        else if junk_roll < 0.30 { ("bone", 1) }
        else if junk_roll < 0.45 { ("string", 1) }
        else if junk_roll < 0.60 { ("rotten_flesh", 1) }
        else if junk_roll < 0.75 { ("bowl", 1) }
        else if junk_roll < 0.90 { ("stick", 1) }
        else { ("ink_sac", 1) }
    } else {
        // Treasure category (simplified)
        let treasure_roll = (roll - 0.95) / 0.05;
        if treasure_roll < 0.33 { ("name_tag", 1) }
        else if treasure_roll < 0.66 { ("saddle", 1) }
        else { ("nautilus_shell", 1) }
    }
}

// === Farming Data ===

// Farmland block state IDs (moisture 0-7)
const FARMLAND_MIN: i32 = 4286;
const FARMLAND_MAX: i32 = 4293;

// Crop block state ranges (age property)
const WHEAT_MIN: i32 = 4278;
const WHEAT_MAX: i32 = 4285;     // age 0-7
const CARROTS_MIN: i32 = 8595;
const CARROTS_MAX: i32 = 8602;   // age 0-7
const POTATOES_MIN: i32 = 8603;
const POTATOES_MAX: i32 = 8610;  // age 0-7
const BEETROOTS_MIN: i32 = 12509;
const BEETROOTS_MAX: i32 = 12512; // age 0-3

/// Returns true if the block state is farmland.
pub fn is_farmland(state_id: i32) -> bool {
    (FARMLAND_MIN..=FARMLAND_MAX).contains(&state_id)
}

/// Returns the farmland moisture level (0-7), or None if not farmland.
pub fn farmland_moisture(state_id: i32) -> Option<i32> {
    if is_farmland(state_id) {
        Some(state_id - FARMLAND_MIN)
    } else {
        None
    }
}

/// Returns the farmland block state for a given moisture level (0-7).
pub fn farmland_state(moisture: i32) -> i32 {
    FARMLAND_MIN + moisture.clamp(0, 7)
}

/// Returns true if the block state is any crop block.
pub fn is_crop(state_id: i32) -> bool {
    (WHEAT_MIN..=WHEAT_MAX).contains(&state_id)
        || (CARROTS_MIN..=CARROTS_MAX).contains(&state_id)
        || (POTATOES_MIN..=POTATOES_MAX).contains(&state_id)
        || (BEETROOTS_MIN..=BEETROOTS_MAX).contains(&state_id)
}

/// Returns the crop age and max age for a crop block state, or None.
pub fn crop_age(state_id: i32) -> Option<(i32, i32)> {
    if (WHEAT_MIN..=WHEAT_MAX).contains(&state_id) {
        Some((state_id - WHEAT_MIN, 7))
    } else if (CARROTS_MIN..=CARROTS_MAX).contains(&state_id) {
        Some((state_id - CARROTS_MIN, 7))
    } else if (POTATOES_MIN..=POTATOES_MAX).contains(&state_id) {
        Some((state_id - POTATOES_MIN, 7))
    } else if (BEETROOTS_MIN..=BEETROOTS_MAX).contains(&state_id) {
        Some((state_id - BEETROOTS_MIN, 3))
    } else {
        None
    }
}

/// Advance a crop's age by the given amount, clamping to max age.
/// Returns the new block state, or None if not a crop.
pub fn crop_grow(state_id: i32, stages: i32) -> Option<i32> {
    if (WHEAT_MIN..=WHEAT_MAX).contains(&state_id) {
        let age = (state_id - WHEAT_MIN + stages).min(7);
        Some(WHEAT_MIN + age)
    } else if (CARROTS_MIN..=CARROTS_MAX).contains(&state_id) {
        let age = (state_id - CARROTS_MIN + stages).min(7);
        Some(CARROTS_MIN + age)
    } else if (POTATOES_MIN..=POTATOES_MAX).contains(&state_id) {
        let age = (state_id - POTATOES_MIN + stages).min(7);
        Some(POTATOES_MIN + age)
    } else if (BEETROOTS_MIN..=BEETROOTS_MAX).contains(&state_id) {
        let age = (state_id - BEETROOTS_MIN + stages).min(3);
        Some(BEETROOTS_MIN + age)
    } else {
        None
    }
}

/// Returns the seed/planting item ID for a given crop seed item,
/// and the initial crop block state to place.
/// Returns None if the item is not a plantable crop seed.
pub fn seed_to_crop(item_name: &str) -> Option<i32> {
    match item_name {
        "wheat_seeds" => Some(WHEAT_MIN),       // wheat age=0
        "carrot" => Some(CARROTS_MIN),          // carrots age=0
        "potato" => Some(POTATOES_MIN),         // potatoes age=0
        "beetroot_seeds" => Some(BEETROOTS_MIN), // beetroots age=0
        _ => None,
    }
}

/// Returns crop drops as (item_name, min_count, max_count, seed_name, seed_min, seed_max).
/// Only drops full items at max age.
pub fn crop_drops(state_id: i32) -> Option<(&'static str, i32, i32, &'static str, i32, i32)> {
    let (age, max_age) = crop_age(state_id)?;
    if (WHEAT_MIN..=WHEAT_MAX).contains(&state_id) {
        if age >= max_age {
            Some(("wheat", 1, 1, "wheat_seeds", 1, 3))
        } else {
            Some(("wheat_seeds", 1, 1, "", 0, 0))
        }
    } else if (CARROTS_MIN..=CARROTS_MAX).contains(&state_id) {
        if age >= max_age {
            Some(("carrot", 1, 4, "", 0, 0))
        } else {
            Some(("carrot", 1, 1, "", 0, 0))
        }
    } else if (POTATOES_MIN..=POTATOES_MAX).contains(&state_id) {
        if age >= max_age {
            Some(("potato", 1, 4, "", 0, 0))
        } else {
            Some(("potato", 1, 1, "", 0, 0))
        }
    } else if (BEETROOTS_MIN..=BEETROOTS_MAX).contains(&state_id) {
        if age >= max_age {
            Some(("beetroot", 1, 1, "beetroot_seeds", 1, 3))
        } else {
            Some(("beetroot_seeds", 1, 1, "", 0, 0))
        }
    } else {
        None
    }
}

/// Returns true if a block can be hoed into farmland.
pub fn is_hoeable(block_name: &str) -> bool {
    matches!(block_name, "grass_block" | "dirt" | "dirt_path")
}

/// Returns true if the item is a hoe.
pub fn is_hoe(item_name: &str) -> bool {
    matches!(item_name, "wooden_hoe" | "stone_hoe" | "iron_hoe" | "golden_hoe" | "diamond_hoe" | "netherite_hoe")
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

    #[test]
    fn test_mob_data() {
        assert_eq!(mob_type_name(MOB_PIG), Some("pig"));
        assert_eq!(mob_type_name(MOB_ZOMBIE), Some("zombie"));
        assert_eq!(mob_type_name(999), None);

        assert_eq!(mob_max_health(MOB_CHICKEN), 4.0);
        assert_eq!(mob_max_health(MOB_ZOMBIE), 20.0);

        assert!(mob_is_hostile(MOB_ZOMBIE));
        assert!(!mob_is_hostile(MOB_PIG));

        assert_eq!(mob_attack_damage(MOB_ZOMBIE), 3.0);
        assert_eq!(mob_attack_damage(MOB_COW), 0.0);

        assert!(!mob_drops(MOB_PIG).is_empty());
        assert_eq!(mob_drops(MOB_PIG)[0].0, "porkchop");

        assert_eq!(mob_xp_drop(MOB_ZOMBIE), 5);
        assert_eq!(mob_xp_drop(MOB_COW), 3);

        let (w, h) = mob_hitbox(MOB_ZOMBIE);
        assert!((w - 0.6).abs() < 0.01);
        assert!((h - 1.95).abs() < 0.01);

        let (ambient, hurt, death) = mob_sounds(MOB_COW);
        assert_eq!(ambient, "entity.cow.ambient");
        assert_eq!(hurt, "entity.cow.hurt");
        assert_eq!(death, "entity.cow.death");

        // New mob types
        assert_eq!(mob_type_name(MOB_SKELETON), Some("skeleton"));
        assert_eq!(mob_type_name(MOB_SPIDER), Some("spider"));
        assert_eq!(mob_type_name(MOB_CREEPER), Some("creeper"));
        assert_eq!(mob_type_name(MOB_ENDERMAN), Some("enderman"));
        assert_eq!(mob_type_name(MOB_SLIME), Some("slime"));
        assert_eq!(mob_type_name(MOB_BAT), Some("bat"));

        assert!(mob_is_hostile(MOB_SKELETON));
        assert!(mob_is_hostile(MOB_SPIDER));
        assert!(mob_is_hostile(MOB_CREEPER));
        assert!(!mob_is_hostile(MOB_BAT));

        assert_eq!(mob_max_health(MOB_ENDERMAN), 40.0);
        assert_eq!(mob_attack_damage(MOB_ENDERMAN), 7.0);

        assert!(mob_is_ranged(MOB_SKELETON));
        assert!(!mob_is_ranged(MOB_ZOMBIE));
        assert!(mob_is_explosive(MOB_CREEPER));
    }
}

// ── Status Effects ───────────────────────────────────────────────────

/// Returns the registry ID (0-indexed) for a named effect, or None.
pub fn effect_name_to_id(name: &str) -> Option<i32> {
    match name {
        "speed" => Some(0),
        "slowness" => Some(1),
        "haste" => Some(2),
        "mining_fatigue" => Some(3),
        "strength" => Some(4),
        "instant_health" => Some(5),
        "instant_damage" => Some(6),
        "jump_boost" => Some(7),
        "nausea" => Some(8),
        "regeneration" => Some(9),
        "resistance" => Some(10),
        "fire_resistance" => Some(11),
        "water_breathing" => Some(12),
        "invisibility" => Some(13),
        "blindness" => Some(14),
        "night_vision" => Some(15),
        "hunger" => Some(16),
        "weakness" => Some(17),
        "poison" => Some(18),
        "wither" => Some(19),
        "health_boost" => Some(20),
        "absorption" => Some(21),
        "saturation" => Some(22),
        "glowing" => Some(23),
        "levitation" => Some(24),
        "luck" => Some(25),
        "unluck" => Some(26),
        "slow_falling" => Some(27),
        "conduit_power" => Some(28),
        "dolphins_grace" => Some(29),
        "bad_omen" => Some(30),
        "hero_of_the_village" => Some(31),
        "darkness" => Some(32),
        _ => None,
    }
}

/// Returns the name for a given effect registry ID.
pub fn effect_id_to_name(id: i32) -> Option<&'static str> {
    match id {
        0 => Some("speed"),
        1 => Some("slowness"),
        2 => Some("haste"),
        3 => Some("mining_fatigue"),
        4 => Some("strength"),
        5 => Some("instant_health"),
        6 => Some("instant_damage"),
        7 => Some("jump_boost"),
        8 => Some("nausea"),
        9 => Some("regeneration"),
        10 => Some("resistance"),
        11 => Some("fire_resistance"),
        12 => Some("water_breathing"),
        13 => Some("invisibility"),
        14 => Some("blindness"),
        15 => Some("night_vision"),
        16 => Some("hunger"),
        17 => Some("weakness"),
        18 => Some("poison"),
        19 => Some("wither"),
        20 => Some("health_boost"),
        21 => Some("absorption"),
        22 => Some("saturation"),
        23 => Some("glowing"),
        24 => Some("levitation"),
        25 => Some("luck"),
        26 => Some("unluck"),
        27 => Some("slow_falling"),
        28 => Some("conduit_power"),
        29 => Some("dolphins_grace"),
        30 => Some("bad_omen"),
        31 => Some("hero_of_the_village"),
        32 => Some("darkness"),
        _ => None,
    }
}

// ── Potions ──────────────────────────────────────────────────────────

/// A potion effect entry: (effect_id, duration_ticks, amplifier).
pub struct PotionEffect {
    pub effect_id: i32,
    pub duration: i32,
    pub amplifier: i32,
}

/// Returns the potion type index for a given potion name, or None.
/// This index is stored in ItemStack.damage for potion items.
pub fn potion_name_to_index(name: &str) -> Option<i32> {
    match name {
        "water" => Some(0),
        "mundane" => Some(1),
        "thick" => Some(2),
        "awkward" => Some(3),
        "night_vision" => Some(4),
        "long_night_vision" => Some(5),
        "invisibility" => Some(6),
        "long_invisibility" => Some(7),
        "leaping" => Some(8),
        "long_leaping" => Some(9),
        "strong_leaping" => Some(10),
        "fire_resistance" => Some(11),
        "long_fire_resistance" => Some(12),
        "swiftness" => Some(13),
        "long_swiftness" => Some(14),
        "strong_swiftness" => Some(15),
        "slowness" => Some(16),
        "long_slowness" => Some(17),
        "strong_slowness" => Some(18),
        "water_breathing" => Some(19),
        "long_water_breathing" => Some(20),
        "healing" => Some(21),
        "strong_healing" => Some(22),
        "harming" => Some(23),
        "strong_harming" => Some(24),
        "poison" => Some(25),
        "long_poison" => Some(26),
        "strong_poison" => Some(27),
        "regeneration" => Some(28),
        "long_regeneration" => Some(29),
        "strong_regeneration" => Some(30),
        "strength" => Some(31),
        "long_strength" => Some(32),
        "strong_strength" => Some(33),
        "weakness" => Some(34),
        "long_weakness" => Some(35),
        "luck" => Some(36),
        "slow_falling" => Some(37),
        "long_slow_falling" => Some(38),
        _ => None,
    }
}

/// Returns the potion name for a given index.
pub fn potion_index_to_name(index: i32) -> Option<&'static str> {
    match index {
        0 => Some("water"),
        1 => Some("mundane"),
        2 => Some("thick"),
        3 => Some("awkward"),
        4 => Some("night_vision"),
        5 => Some("long_night_vision"),
        6 => Some("invisibility"),
        7 => Some("long_invisibility"),
        8 => Some("leaping"),
        9 => Some("long_leaping"),
        10 => Some("strong_leaping"),
        11 => Some("fire_resistance"),
        12 => Some("long_fire_resistance"),
        13 => Some("swiftness"),
        14 => Some("long_swiftness"),
        15 => Some("strong_swiftness"),
        16 => Some("slowness"),
        17 => Some("long_slowness"),
        18 => Some("strong_slowness"),
        19 => Some("water_breathing"),
        20 => Some("long_water_breathing"),
        21 => Some("healing"),
        22 => Some("strong_healing"),
        23 => Some("harming"),
        24 => Some("strong_harming"),
        25 => Some("poison"),
        26 => Some("long_poison"),
        27 => Some("strong_poison"),
        28 => Some("regeneration"),
        29 => Some("long_regeneration"),
        30 => Some("strong_regeneration"),
        31 => Some("strength"),
        32 => Some("long_strength"),
        33 => Some("strong_strength"),
        34 => Some("weakness"),
        35 => Some("long_weakness"),
        36 => Some("luck"),
        37 => Some("slow_falling"),
        38 => Some("long_slow_falling"),
        _ => None,
    }
}

/// Returns the effects for a given potion type index.
/// Returns empty vec for water/mundane/thick/awkward (no effects).
pub fn potion_effects(index: i32) -> Vec<PotionEffect> {
    match index {
        // No-effect potions
        0..=3 => vec![],
        // Night vision
        4 => vec![PotionEffect { effect_id: 15, duration: 3600, amplifier: 0 }],
        5 => vec![PotionEffect { effect_id: 15, duration: 9600, amplifier: 0 }],
        // Invisibility
        6 => vec![PotionEffect { effect_id: 13, duration: 3600, amplifier: 0 }],
        7 => vec![PotionEffect { effect_id: 13, duration: 9600, amplifier: 0 }],
        // Leaping (jump_boost=7)
        8 => vec![PotionEffect { effect_id: 7, duration: 3600, amplifier: 0 }],
        9 => vec![PotionEffect { effect_id: 7, duration: 9600, amplifier: 0 }],
        10 => vec![PotionEffect { effect_id: 7, duration: 1800, amplifier: 1 }],
        // Fire resistance
        11 => vec![PotionEffect { effect_id: 11, duration: 3600, amplifier: 0 }],
        12 => vec![PotionEffect { effect_id: 11, duration: 9600, amplifier: 0 }],
        // Swiftness (speed=0)
        13 => vec![PotionEffect { effect_id: 0, duration: 3600, amplifier: 0 }],
        14 => vec![PotionEffect { effect_id: 0, duration: 9600, amplifier: 0 }],
        15 => vec![PotionEffect { effect_id: 0, duration: 1800, amplifier: 1 }],
        // Slowness (slowness=1)
        16 => vec![PotionEffect { effect_id: 1, duration: 1800, amplifier: 0 }],
        17 => vec![PotionEffect { effect_id: 1, duration: 4800, amplifier: 0 }],
        18 => vec![PotionEffect { effect_id: 1, duration: 400, amplifier: 3 }],
        // Water breathing
        19 => vec![PotionEffect { effect_id: 12, duration: 3600, amplifier: 0 }],
        20 => vec![PotionEffect { effect_id: 12, duration: 9600, amplifier: 0 }],
        // Healing (instant_health=5)
        21 => vec![PotionEffect { effect_id: 5, duration: 1, amplifier: 0 }],
        22 => vec![PotionEffect { effect_id: 5, duration: 1, amplifier: 1 }],
        // Harming (instant_damage=6)
        23 => vec![PotionEffect { effect_id: 6, duration: 1, amplifier: 0 }],
        24 => vec![PotionEffect { effect_id: 6, duration: 1, amplifier: 1 }],
        // Poison
        25 => vec![PotionEffect { effect_id: 18, duration: 900, amplifier: 0 }],
        26 => vec![PotionEffect { effect_id: 18, duration: 1800, amplifier: 0 }],
        27 => vec![PotionEffect { effect_id: 18, duration: 432, amplifier: 1 }],
        // Regeneration
        28 => vec![PotionEffect { effect_id: 9, duration: 900, amplifier: 0 }],
        29 => vec![PotionEffect { effect_id: 9, duration: 1800, amplifier: 0 }],
        30 => vec![PotionEffect { effect_id: 9, duration: 450, amplifier: 1 }],
        // Strength (strength=4)
        31 => vec![PotionEffect { effect_id: 4, duration: 3600, amplifier: 0 }],
        32 => vec![PotionEffect { effect_id: 4, duration: 9600, amplifier: 0 }],
        33 => vec![PotionEffect { effect_id: 4, duration: 1800, amplifier: 1 }],
        // Weakness
        34 => vec![PotionEffect { effect_id: 17, duration: 1800, amplifier: 0 }],
        35 => vec![PotionEffect { effect_id: 17, duration: 4800, amplifier: 0 }],
        // Luck
        36 => vec![PotionEffect { effect_id: 25, duration: 6000, amplifier: 0 }],
        // Slow falling
        37 => vec![PotionEffect { effect_id: 27, duration: 1800, amplifier: 0 }],
        38 => vec![PotionEffect { effect_id: 27, duration: 4800, amplifier: 0 }],
        _ => vec![],
    }
}

/// Returns true if the given item_id is a drinkable potion.
pub fn is_potion(item_id: i32) -> bool {
    let name = match item_id_to_name(item_id) {
        Some(n) => n,
        None => return false,
    };
    matches!(name, "potion" | "splash_potion" | "lingering_potion")
}

/// Returns true if the item is a valid brewing ingredient/reagent.
pub fn is_brewing_ingredient(item_name: &str) -> bool {
    matches!(item_name,
        "nether_wart" | "glowstone_dust" | "redstone" | "fermented_spider_eye"
        | "golden_carrot" | "magma_cream" | "rabbit_foot" | "sugar"
        | "glistering_melon_slice" | "spider_eye" | "ghast_tear" | "blaze_powder"
        | "pufferfish" | "phantom_membrane" | "turtle_helmet"
        | "gunpowder" | "dragon_breath"
    )
}

/// Brewing recipe: (input_potion_index, ingredient_name) -> output_potion_index.
/// Returns None if no recipe exists.
pub fn brewing_recipe(input_potion_index: i32, ingredient_name: &str) -> Option<i32> {
    match (input_potion_index, ingredient_name) {
        // Base potions from water (index 0)
        (0, "nether_wart") => Some(3),        // water -> awkward
        (0, "glowstone_dust") => Some(2),      // water -> thick
        (0, "redstone") => Some(1),            // water -> mundane
        (0, "fermented_spider_eye") => Some(34), // water -> weakness

        // From awkward (index 3)
        (3, "golden_carrot") => Some(4),       // awkward -> night_vision
        (3, "rabbit_foot") => Some(8),         // awkward -> leaping
        (3, "magma_cream") => Some(11),        // awkward -> fire_resistance
        (3, "sugar") => Some(13),              // awkward -> swiftness
        (3, "pufferfish") => Some(19),         // awkward -> water_breathing
        (3, "glistering_melon_slice") => Some(21), // awkward -> healing
        (3, "spider_eye") => Some(25),         // awkward -> poison
        (3, "ghast_tear") => Some(28),         // awkward -> regeneration
        (3, "blaze_powder") => Some(31),       // awkward -> strength
        (3, "phantom_membrane") => Some(37),   // awkward -> slow_falling
        (3, "turtle_helmet") => Some(4),       // awkward -> night_vision (turtle master not in our potion list)

        // Duration extensions (redstone)
        (4, "redstone") => Some(5),            // night_vision -> long_night_vision
        (6, "redstone") => Some(7),            // invisibility -> long_invisibility
        (8, "redstone") => Some(9),            // leaping -> long_leaping
        (11, "redstone") => Some(12),          // fire_resistance -> long_fire_resistance
        (13, "redstone") => Some(14),          // swiftness -> long_swiftness
        (16, "redstone") => Some(17),          // slowness -> long_slowness
        (19, "redstone") => Some(20),          // water_breathing -> long_water_breathing
        (25, "redstone") => Some(26),          // poison -> long_poison
        (28, "redstone") => Some(29),          // regeneration -> long_regeneration
        (31, "redstone") => Some(32),          // strength -> long_strength
        (34, "redstone") => Some(35),          // weakness -> long_weakness
        (37, "redstone") => Some(38),          // slow_falling -> long_slow_falling

        // Potency upgrades (glowstone_dust)
        (8, "glowstone_dust") => Some(10),     // leaping -> strong_leaping
        (13, "glowstone_dust") => Some(15),    // swiftness -> strong_swiftness
        (16, "glowstone_dust") => Some(18),    // slowness -> strong_slowness
        (21, "glowstone_dust") => Some(22),    // healing -> strong_healing
        (23, "glowstone_dust") => Some(24),    // harming -> strong_harming
        (25, "glowstone_dust") => Some(27),    // poison -> strong_poison
        (28, "glowstone_dust") => Some(30),    // regeneration -> strong_regeneration
        (31, "glowstone_dust") => Some(33),    // strength -> strong_strength

        // Corruption (fermented_spider_eye)
        (4, "fermented_spider_eye") => Some(6),  // night_vision -> invisibility
        (5, "fermented_spider_eye") => Some(7),  // long_night_vision -> long_invisibility
        (8, "fermented_spider_eye") => Some(16), // leaping -> slowness
        (13, "fermented_spider_eye") => Some(16), // swiftness -> slowness
        (14, "fermented_spider_eye") => Some(17), // long_swiftness -> long_slowness
        (25, "fermented_spider_eye") => Some(23), // poison -> harming
        (26, "fermented_spider_eye") => Some(23), // long_poison -> harming
        (27, "fermented_spider_eye") => Some(24), // strong_poison -> strong_harming
        (21, "fermented_spider_eye") => Some(23), // healing -> harming
        (22, "fermented_spider_eye") => Some(24), // strong_healing -> strong_harming

        _ => None,
    }
}

/// Returns the enchantment registry ID for the given name.
/// IDs match the order in the enchantment registry (0-indexed).
pub fn enchantment_name_to_id(name: &str) -> Option<i32> {
    match name {
        "protection" => Some(0),
        "fire_protection" => Some(1),
        "feather_falling" => Some(2),
        "blast_protection" => Some(3),
        "projectile_protection" => Some(4),
        "respiration" => Some(5),
        "aqua_affinity" => Some(6),
        "thorns" => Some(7),
        "depth_strider" => Some(8),
        "frost_walker" => Some(9),
        "binding_curse" => Some(10),
        "soul_speed" => Some(11),
        "swift_sneak" => Some(12),
        "sharpness" => Some(13),
        "smite" => Some(14),
        "bane_of_arthropods" => Some(15),
        "knockback" => Some(16),
        "fire_aspect" => Some(17),
        "looting" => Some(18),
        "sweeping_edge" => Some(19),
        "efficiency" => Some(20),
        "silk_touch" => Some(21),
        "unbreaking" => Some(22),
        "fortune" => Some(23),
        "power" => Some(24),
        "punch" => Some(25),
        "flame" => Some(26),
        "infinity" => Some(27),
        "luck_of_the_sea" => Some(28),
        "lure" => Some(29),
        "loyalty" => Some(30),
        "impaling" => Some(31),
        "riptide" => Some(32),
        "channeling" => Some(33),
        "multishot" => Some(34),
        "quick_charge" => Some(35),
        "piercing" => Some(36),
        "mending" => Some(37),
        "vanishing_curse" => Some(38),
        "density" => Some(39),
        "breach" => Some(40),
        "wind_burst" => Some(41),
        _ => None,
    }
}

/// Returns the enchantment name for the given registry ID.
pub fn enchantment_id_to_name(id: i32) -> Option<&'static str> {
    match id {
        0 => Some("protection"),
        1 => Some("fire_protection"),
        2 => Some("feather_falling"),
        3 => Some("blast_protection"),
        4 => Some("projectile_protection"),
        5 => Some("respiration"),
        6 => Some("aqua_affinity"),
        7 => Some("thorns"),
        8 => Some("depth_strider"),
        9 => Some("frost_walker"),
        10 => Some("binding_curse"),
        11 => Some("soul_speed"),
        12 => Some("swift_sneak"),
        13 => Some("sharpness"),
        14 => Some("smite"),
        15 => Some("bane_of_arthropods"),
        16 => Some("knockback"),
        17 => Some("fire_aspect"),
        18 => Some("looting"),
        19 => Some("sweeping_edge"),
        20 => Some("efficiency"),
        21 => Some("silk_touch"),
        22 => Some("unbreaking"),
        23 => Some("fortune"),
        24 => Some("power"),
        25 => Some("punch"),
        26 => Some("flame"),
        27 => Some("infinity"),
        28 => Some("luck_of_the_sea"),
        29 => Some("lure"),
        30 => Some("loyalty"),
        31 => Some("impaling"),
        32 => Some("riptide"),
        33 => Some("channeling"),
        34 => Some("multishot"),
        35 => Some("quick_charge"),
        36 => Some("piercing"),
        37 => Some("mending"),
        38 => Some("vanishing_curse"),
        39 => Some("density"),
        40 => Some("breach"),
        41 => Some("wind_burst"),
        _ => None,
    }
}

/// Returns the maximum level for the given enchantment.
pub fn enchantment_max_level(id: i32) -> i32 {
    match id {
        0..=4 => 4,   // protection types
        5 => 3,        // respiration
        6 => 1,        // aqua_affinity
        7 => 3,        // thorns
        8 => 3,        // depth_strider
        9 => 2,        // frost_walker
        10 => 1,       // binding_curse
        11 => 3,       // soul_speed
        12 => 3,       // swift_sneak
        13..=15 => 5,  // sharpness, smite, bane_of_arthropods
        16 => 2,       // knockback
        17 => 2,       // fire_aspect
        18 => 3,       // looting
        19 => 3,       // sweeping_edge
        20 => 5,       // efficiency
        21 => 1,       // silk_touch
        22 => 3,       // unbreaking
        23 => 3,       // fortune
        24 => 5,       // power
        25 => 2,       // punch
        26 => 1,       // flame
        27 => 1,       // infinity
        28 => 3,       // luck_of_the_sea
        29 => 3,       // lure
        30 => 3,       // loyalty
        31 => 5,       // impaling
        32 => 3,       // riptide
        33 => 1,       // channeling
        34 => 1,       // multishot
        35 => 3,       // quick_charge
        36 => 4,       // piercing
        37 => 1,       // mending
        38 => 1,       // vanishing_curse
        39 => 5,       // density
        40 => 4,       // breach
        41 => 3,       // wind_burst
        _ => 0,
    }
}

/// Returns the anvil XP cost multiplier per level for the given enchantment.
/// Returns true if the given block name is a standing sign (any wood type).
pub fn is_standing_sign(name: &str) -> bool {
    matches!(name,
        "oak_sign" | "spruce_sign" | "birch_sign" | "acacia_sign" | "cherry_sign"
        | "jungle_sign" | "dark_oak_sign" | "mangrove_sign" | "bamboo_sign"
        | "crimson_sign" | "warped_sign"
    )
}

/// Returns true if the given block name is a wall sign (any wood type).
pub fn is_wall_sign(name: &str) -> bool {
    matches!(name,
        "oak_wall_sign" | "spruce_wall_sign" | "birch_wall_sign" | "acacia_wall_sign"
        | "cherry_wall_sign" | "jungle_wall_sign" | "dark_oak_wall_sign"
        | "mangrove_wall_sign" | "bamboo_wall_sign"
        | "crimson_wall_sign" | "warped_wall_sign"
    )
}

/// Returns true if the given block name is any kind of sign (standing or wall).
pub fn is_any_sign(name: &str) -> bool {
    is_standing_sign(name) || is_wall_sign(name)
}

/// Returns true if the given block state ID belongs to a sign block.
pub fn is_sign_state(state_id: i32) -> bool {
    block_state_to_name(state_id).map(|n| is_any_sign(n)).unwrap_or(false)
}

/// Given a standing sign's item name (e.g. "oak_sign"), returns (standing_min_state, wall_min_state).
pub fn sign_state_ids(item_name: &str) -> Option<(i32, i32)> {
    match item_name {
        "oak_sign" => Some((4302, 4762)),
        "spruce_sign" => Some((4334, 4770)),
        "birch_sign" => Some((4366, 4778)),
        "acacia_sign" => Some((4398, 4786)),
        "cherry_sign" => Some((4430, 4794)),
        "jungle_sign" => Some((4462, 4802)),
        "dark_oak_sign" => Some((4494, 4810)),
        "mangrove_sign" => Some((4526, 4818)),
        "bamboo_sign" => Some((4558, 4826)),
        "crimson_sign" => Some((19276, 19340)),
        "warped_sign" => Some((19308, 19348)),
        _ => None,
    }
}

/// Compute block state for a standing sign given its min state and player yaw.
/// Standing sign rotation: 16 directions (0-15), each with waterlogged variant.
/// State layout: minState + rotation * 2 + waterlogged(0/1)
pub fn standing_sign_state(min_state: i32, yaw: f32) -> i32 {
    let rotation = yaw_to_sign_rotation(yaw);
    min_state + rotation * 2 // waterlogged=false
}

/// Compute block state for a wall sign given its min state and block face.
/// Wall sign facing: north=0, south=1, west=2, east=3, each with waterlogged variant.
/// State layout: minState + facing * 2 + waterlogged(0/1)
/// Face: 2=north, 3=south, 4=west, 5=east
pub fn wall_sign_state(min_state: i32, face: u8) -> i32 {
    let facing = match face {
        2 => 0, // north
        3 => 1, // south
        4 => 2, // west
        5 => 3, // east
        _ => 0,
    };
    min_state + facing * 2 // waterlogged=false
}

/// Convert player yaw to standing sign rotation (0-15).
/// MC formula: floor((yaw + 180) / 22.5) & 15
pub fn yaw_to_sign_rotation(yaw: f32) -> i32 {
    (((yaw + 180.0) / 22.5).floor() as i32) & 15
}

pub fn enchantment_anvil_cost(id: i32) -> i32 {
    match id {
        0..=4 => 1,   // protection types
        5 => 2,        // respiration
        6 => 2,        // aqua_affinity
        7 => 4,        // thorns
        8 => 2,        // depth_strider
        9 => 2,        // frost_walker
        10 => 4,       // binding_curse
        11 => 4,       // soul_speed
        12 => 4,       // swift_sneak
        13 => 1,       // sharpness
        14 => 1,       // smite
        15 => 1,       // bane_of_arthropods
        16 => 1,       // knockback
        17 => 2,       // fire_aspect
        18 => 2,       // looting
        19 => 2,       // sweeping_edge
        20 => 1,       // efficiency
        21 => 4,       // silk_touch
        22 => 1,       // unbreaking
        23 => 2,       // fortune
        24 => 1,       // power
        25 => 2,       // punch
        26 => 2,       // flame
        27 => 4,       // infinity
        28 => 2,       // luck_of_the_sea
        29 => 2,       // lure
        30 => 1,       // loyalty
        31 => 2,       // impaling
        32 => 2,       // riptide
        33 => 4,       // channeling
        34 => 2,       // multishot
        35 => 1,       // quick_charge
        36 => 1,       // piercing
        37 => 2,       // mending
        38 => 4,       // vanishing_curse
        39 => 2,       // density
        40 => 2,       // breach
        41 => 2,       // wind_burst
        _ => 1,
    }
}
