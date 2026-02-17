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

-- /weather <clear|rain|thunder> [duration] — change weather
pickaxe.commands.register("weather", function(player_name, args)
    local parts = {}
    for word in args:gmatch("%S+") do
        table.insert(parts, word)
    end

    local weather_type = parts[1]
    if not weather_type then
        local current = pickaxe.world.get_weather()
        pickaxe.players.send_message(player_name, "Current weather: " .. current .. ". Usage: /weather <clear|rain|thunder> [duration]")
        return
    end

    if weather_type ~= "clear" and weather_type ~= "rain" and weather_type ~= "thunder" then
        pickaxe.players.send_message(player_name, "Invalid weather type. Use: clear, rain, or thunder")
        return
    end

    local duration = tonumber(parts[2]) or 6000
    pickaxe.world.set_weather(weather_type, duration)
    pickaxe.players.send_message(player_name, "Weather set to " .. weather_type .. " for " .. duration .. " ticks")
end)
