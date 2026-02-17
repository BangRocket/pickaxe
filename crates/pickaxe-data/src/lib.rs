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

    recipes
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
}
