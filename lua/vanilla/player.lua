-- Player event handlers

-- Player join: welcome message + logging
pickaxe.events.on("player_join", function(event)
    local name = event.name or "unknown"
    pickaxe.log("Player joined: " .. name)
    pickaxe.players.broadcast(name .. " joined the game")
    pickaxe.players.send_message(name, "Welcome to Pickaxe! Type /help for commands.")
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })

-- Player leave: broadcast + logging
pickaxe.events.on("player_leave", function(event)
    local name = event.name or "unknown"
    pickaxe.log("Player left: " .. name)
    pickaxe.players.broadcast(name .. " left the game")
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })

-- Player chat: format and broadcast (cancels default Rust broadcast)
pickaxe.events.on("player_chat", function(event)
    local name = event.name or "?"
    local message = event.message or ""
    pickaxe.players.broadcast("<" .. name .. "> " .. message)
    return "cancel"
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })

-- Player command event (logging only)
pickaxe.events.on("player_command", function(event)
    pickaxe.log(event.name .. " issued command: /" .. event.command)
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })
