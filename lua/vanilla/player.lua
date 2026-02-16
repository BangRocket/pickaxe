-- Player event handlers

-- Player join event
pickaxe.events.on("player_join", function(event)
    pickaxe.log("Player joined: " .. (event.name or "unknown"))
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })

-- Player leave event
pickaxe.events.on("player_leave", function(event)
    pickaxe.log("Player left: " .. (event.name or "unknown"))
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })

-- Player move event
pickaxe.events.on("player_move", function(event)
    if event.x and event.z then
        pickaxe.log("Player " .. (event.name or "?") .. " moved to " .. event.x .. ", " .. event.y .. ", " .. event.z)
    end
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })

-- Player chat event
pickaxe.events.on("player_chat", function(event)
    pickaxe.log("<" .. (event.name or "?") .. "> " .. (event.message or ""))
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })

-- Player command event
pickaxe.events.on("player_command", function(event)
    pickaxe.log(event.name .. " issued command: /" .. event.command)
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })
