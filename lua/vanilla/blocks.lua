-- Block property overrides
-- These override the codegen defaults from pickaxe-data.
-- Any block not registered here uses the codegen fallback.

local pickaxes = {
    "wooden_pickaxe", "stone_pickaxe", "iron_pickaxe",
    "golden_pickaxe", "diamond_pickaxe", "netherite_pickaxe",
}

local shovels = {
    "wooden_shovel", "stone_shovel", "iron_shovel",
    "golden_shovel", "diamond_shovel", "netherite_shovel",
}

local axes = {
    "wooden_axe", "stone_axe", "iron_axe",
    "golden_axe", "diamond_axe", "netherite_axe",
}

-- Stone blocks
pickaxe.blocks.register("stone", {
    hardness = 1.5,
    drops = {"cobblestone"},
    harvest_tools = pickaxes,
})

pickaxe.blocks.register("cobblestone", {
    hardness = 2.0,
    drops = {"cobblestone"},
    harvest_tools = pickaxes,
})

-- Dirt/grass
pickaxe.blocks.register("dirt", {
    hardness = 0.5,
    drops = {"dirt"},
})

pickaxe.blocks.register("grass_block", {
    hardness = 0.6,
    drops = {"dirt"},
})

-- Ores
pickaxe.blocks.register("coal_ore", {
    hardness = 3.0,
    drops = {"coal"},
    harvest_tools = pickaxes,
})

pickaxe.blocks.register("iron_ore", {
    hardness = 3.0,
    drops = {"raw_iron"},
    harvest_tools = pickaxes,
})

pickaxe.blocks.register("gold_ore", {
    hardness = 3.0,
    drops = {"raw_gold"},
    harvest_tools = pickaxes,
})

pickaxe.blocks.register("diamond_ore", {
    hardness = 3.0,
    drops = {"diamond"},
    harvest_tools = pickaxes,
})

pickaxe.blocks.register("lapis_ore", {
    hardness = 3.0,
    drops = {"lapis_lazuli"},
    harvest_tools = pickaxes,
})

pickaxe.blocks.register("redstone_ore", {
    hardness = 3.0,
    drops = {"redstone"},
    harvest_tools = pickaxes,
})

pickaxe.blocks.register("emerald_ore", {
    hardness = 3.0,
    drops = {"emerald"},
    harvest_tools = pickaxes,
})

-- Wood
pickaxe.blocks.register("oak_log", {
    hardness = 2.0,
    drops = {"oak_log"},
    harvest_tools = axes,
})

pickaxe.blocks.register("oak_planks", {
    hardness = 2.0,
    drops = {"oak_planks"},
    harvest_tools = axes,
})

-- Sand/gravel
pickaxe.blocks.register("sand", {
    hardness = 0.5,
    drops = {"sand"},
    harvest_tools = shovels,
})

pickaxe.blocks.register("gravel", {
    hardness = 0.6,
    drops = {"gravel"},
    harvest_tools = shovels,
})

pickaxe.log("Block overrides registered for " .. 15 .. " blocks")
