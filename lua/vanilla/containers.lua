-- Container event handlers

-- Container open: log when a player opens a container
pickaxe.events.on("container_open", function(event)
    pickaxe.log(event.name .. " opened " .. event.block_type .. " at " .. event.x .. "," .. event.y .. "," .. event.z)
end, { priority = "MONITOR", mod_id = "pickaxe-vanilla" })

-- Container close: log when a player closes a container
pickaxe.events.on("container_close", function(event)
    pickaxe.log(event.name .. " closed " .. event.block_type)
end, { priority = "MONITOR", mod_id = "pickaxe-vanilla" })
