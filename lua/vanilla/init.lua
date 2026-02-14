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

pickaxe.log("Pickaxe Vanilla mod loaded - " .. "3 event handlers registered")
