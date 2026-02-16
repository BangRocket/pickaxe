-- World event handlers (blocks, server lifecycle)

-- Server start event
pickaxe.events.on("server_start", function(event)
    pickaxe.log("Server started! Vanilla mod ready.")
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
