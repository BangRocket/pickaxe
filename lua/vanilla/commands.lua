-- Custom commands registered by the vanilla mod

-- /hello — greet the player
pickaxe.commands.register("hello", function(player_name, args)
    pickaxe.players.send_message(player_name, "Hello, " .. player_name .. "! Welcome to Pickaxe.")
end)

-- /spawn — teleport to spawn
pickaxe.commands.register("spawn", function(player_name, args)
    pickaxe.players.teleport(player_name, 0.5, -59.0, 0.5)
    pickaxe.players.send_message(player_name, "Teleported to spawn!")
end)
