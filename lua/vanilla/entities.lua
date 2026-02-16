-- Item entity behavior
-- Default pickup handler (allows pickup by default)

pickaxe.events.on("item_pickup", function(event)
    pickaxe.log(event.name .. " picked up " .. (event.item_name or "item"))
end, { priority = "NORMAL", mod_id = "pickaxe-vanilla" })
