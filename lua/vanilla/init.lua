-- Pickaxe Vanilla Mod
-- Implements vanilla Minecraft game behavior through the event system

pickaxe.log("Pickaxe Vanilla mod loading...")

-- Server start event
pickaxe.events.on("server_start", function(event)
    pickaxe.log("Server started! Vanilla mod ready.")
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })

-- Player join event
pickaxe.events.on("player_join", function(event)
    pickaxe.log("Player joined: " .. (event.name or "unknown"))
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })

-- Player move event
pickaxe.events.on("player_move", function(event)
    -- Only log occasionally to avoid spam
    if event.x and event.z then
        pickaxe.log("Player " .. (event.name or "?") .. " moved to " .. event.x .. ", " .. event.y .. ", " .. event.z)
    end
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })

-- Player chat event
pickaxe.events.on("player_chat", function(event)
    pickaxe.log("<" .. (event.name or "?") .. "> " .. (event.message or ""))
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })

-- Player leave event
pickaxe.events.on("player_leave", function(event)
    pickaxe.log("Player left: " .. (event.name or "unknown"))
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })

-- Player command event
pickaxe.events.on("player_command", function(event)
    pickaxe.log(event.name .. " issued command: /" .. event.command)
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })

-- Block break event
pickaxe.events.on("block_break", function(event)
    pickaxe.log("Block broken at " .. event.x .. "," .. event.y .. "," .. event.z ..
                " by " .. event.name .. " (was block " .. event.block_id .. ")")
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })

-- Block place event
pickaxe.events.on("block_place", function(event)
    pickaxe.log("Block placed at " .. event.x .. "," .. event.y .. "," .. event.z ..
                " by " .. event.name .. " (block " .. event.block_id .. ")")
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })

pickaxe.log("Pickaxe Vanilla mod loaded - " .. "8 event handlers registered")
