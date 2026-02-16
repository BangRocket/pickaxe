-- Pickaxe Vanilla Mod
-- Implements vanilla Minecraft game behavior through the event system

pickaxe.log("Pickaxe Vanilla mod loading...")

-- Load domain modules
dofile("lua/vanilla/player.lua")
dofile("lua/vanilla/world.lua")
dofile("lua/vanilla/commands.lua")

pickaxe.log("Pickaxe Vanilla mod loaded - player, world, and command handlers registered")
