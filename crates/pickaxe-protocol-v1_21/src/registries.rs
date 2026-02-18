use pickaxe_nbt::{nbt_compound, nbt_list, NbtValue};
use pickaxe_protocol_core::{InternalPacket, RegistryEntry};

/// Build all required registry data packets for MC 1.21 Configuration state.
/// The client expects specific registries to be sent during configuration.
pub fn build_registry_packets() -> Vec<InternalPacket> {
    vec![
        build_dimension_type_registry(),
        build_biome_registry(),
        build_chat_type_registry(),
        build_trim_pattern_registry(),
        build_trim_material_registry(),
        build_wolf_variant_registry(),
        build_painting_variant_registry(),
        build_damage_type_registry(),
        build_banner_pattern_registry(),
        build_enchantment_registry(),
        build_jukebox_song_registry(),
    ]
}

fn build_dimension_type_registry() -> InternalPacket {
    InternalPacket::RegistryData {
        registry_id: "minecraft:dimension_type".into(),
        entries: vec![RegistryEntry {
            id: "minecraft:overworld".into(),
            data: Some(nbt_compound! {
                "has_skylight" => NbtValue::Byte(1),
                "has_ceiling" => NbtValue::Byte(0),
                "ultrawarm" => NbtValue::Byte(0),
                "natural" => NbtValue::Byte(1),
                "coordinate_scale" => NbtValue::Double(1.0),
                "bed_works" => NbtValue::Byte(1),
                "respawn_anchor_works" => NbtValue::Byte(0),
                "min_y" => NbtValue::Int(-64),
                "height" => NbtValue::Int(384),
                "logical_height" => NbtValue::Int(384),
                "infiniburn" => NbtValue::String("#minecraft:infiniburn_overworld".into()),
                "effects" => NbtValue::String("minecraft:overworld".into()),
                "ambient_light" => NbtValue::Float(0.0),
                "piglin_safe" => NbtValue::Byte(0),
                "has_raids" => NbtValue::Byte(1),
                "monster_spawn_light_level" => NbtValue::Int(0),
                "monster_spawn_block_light_limit" => NbtValue::Int(0)
            }),
        }],
    }
}

fn build_biome_registry() -> InternalPacket {
    InternalPacket::RegistryData {
        registry_id: "minecraft:worldgen/biome".into(),
        entries: vec![RegistryEntry {
            id: "minecraft:plains".into(),
            data: Some(nbt_compound! {
                "has_precipitation" => NbtValue::Byte(1),
                "temperature" => NbtValue::Float(0.8),
                "downfall" => NbtValue::Float(0.4),
                "effects" => NbtValue::Compound(vec![
                    ("fog_color".into(), NbtValue::Int(12638463)),
                    ("water_color".into(), NbtValue::Int(4159204)),
                    ("water_fog_color".into(), NbtValue::Int(329011)),
                    ("sky_color".into(), NbtValue::Int(7907327)),
                    ("mood_sound".into(), NbtValue::Compound(vec![
                        ("sound".into(), NbtValue::String("minecraft:ambient.cave".into())),
                        ("tick_delay".into(), NbtValue::Int(6000)),
                        ("offset".into(), NbtValue::Double(2.0)),
                        ("block_search_extent".into(), NbtValue::Int(8)),
                    ])),
                ])
            }),
        }],
    }
}

fn build_chat_type_registry() -> InternalPacket {
    InternalPacket::RegistryData {
        registry_id: "minecraft:chat_type".into(),
        entries: vec![RegistryEntry {
            id: "minecraft:chat".into(),
            data: Some(nbt_compound! {
                "chat" => NbtValue::Compound(vec![
                    ("translation_key".into(), NbtValue::String("chat.type.text".into())),
                    ("parameters".into(), nbt_list![
                        NbtValue::String("sender".into()),
                        NbtValue::String("content".into())
                    ]),
                ]),
                "narration" => NbtValue::Compound(vec![
                    ("translation_key".into(), NbtValue::String("chat.type.text.narrate".into())),
                    ("parameters".into(), nbt_list![
                        NbtValue::String("sender".into()),
                        NbtValue::String("content".into())
                    ]),
                ])
            }),
        }],
    }
}

fn damage_entry(id: &str, message_id: &str, scaling: &str, exhaustion: f32,
                 effects: Option<&str>, death_message_type: Option<&str>) -> RegistryEntry {
    let mut fields = vec![
        ("message_id".into(), NbtValue::String(message_id.into())),
        ("scaling".into(), NbtValue::String(scaling.into())),
        ("exhaustion".into(), NbtValue::Float(exhaustion)),
    ];
    if let Some(fx) = effects {
        fields.push(("effects".into(), NbtValue::String(fx.into())));
    }
    if let Some(dmt) = death_message_type {
        fields.push(("death_message_type".into(), NbtValue::String(dmt.into())));
    }
    RegistryEntry {
        id: format!("minecraft:{}", id),
        data: Some(NbtValue::Compound(fields)),
    }
}

fn build_damage_type_registry() -> InternalPacket {
    let s = "when_caused_by_living_non_player";
    InternalPacket::RegistryData {
        registry_id: "minecraft:damage_type".into(),
        entries: vec![
            damage_entry("arrow", "arrow", s, 0.1, None, None),
            damage_entry("bad_respawn_point", "badRespawnPoint", "always", 0.1, None, Some("intentional_game_design")),
            damage_entry("cactus", "cactus", s, 0.1, None, None),
            damage_entry("campfire", "inFire", s, 0.1, Some("burning"), None),
            damage_entry("cramming", "cramming", s, 0.0, None, None),
            damage_entry("dragon_breath", "dragonBreath", s, 0.0, None, None),
            damage_entry("drown", "drown", s, 0.0, Some("drowning"), None),
            damage_entry("dry_out", "dryout", s, 0.1, None, None),
            damage_entry("explosion", "explosion", "always", 0.1, None, None),
            damage_entry("fall", "fall", s, 0.0, None, Some("fall_variants")),
            damage_entry("falling_anvil", "anvil", s, 0.1, None, None),
            damage_entry("falling_block", "fallingBlock", s, 0.1, None, None),
            damage_entry("falling_stalactite", "fallingStalactite", s, 0.1, None, None),
            damage_entry("fireball", "fireball", s, 0.1, Some("burning"), None),
            damage_entry("fireworks", "fireworks", s, 0.1, None, None),
            damage_entry("fly_into_wall", "flyIntoWall", s, 0.0, None, None),
            damage_entry("freeze", "freeze", s, 0.0, Some("freezing"), None),
            damage_entry("generic", "generic", s, 0.0, None, None),
            damage_entry("generic_kill", "genericKill", s, 0.0, None, None),
            damage_entry("hot_floor", "hotFloor", s, 0.1, Some("burning"), None),
            damage_entry("in_fire", "inFire", s, 0.1, Some("burning"), None),
            damage_entry("in_wall", "inWall", s, 0.0, None, None),
            damage_entry("indirect_magic", "indirectMagic", s, 0.0, None, None),
            damage_entry("lava", "lava", s, 0.1, Some("burning"), None),
            damage_entry("lightning_bolt", "lightningBolt", s, 0.1, None, None),
            damage_entry("magic", "magic", s, 0.0, None, None),
            damage_entry("mob_attack", "mob", s, 0.1, None, None),
            damage_entry("mob_attack_no_aggro", "mob", s, 0.1, None, None),
            damage_entry("mob_projectile", "mob", s, 0.1, None, None),
            damage_entry("on_fire", "onFire", s, 0.0, Some("burning"), None),
            damage_entry("out_of_world", "outOfWorld", s, 0.0, None, None),
            damage_entry("outside_border", "outsideBorder", s, 0.0, None, None),
            damage_entry("player_attack", "player", s, 0.1, None, None),
            damage_entry("player_explosion", "explosion.player", "always", 0.1, None, None),
            damage_entry("sonic_boom", "sonic_boom", "always", 0.0, None, None),
            damage_entry("spit", "mob", s, 0.1, None, None),
            damage_entry("stalagmite", "stalagmite", s, 0.0, None, None),
            damage_entry("starve", "starve", s, 0.0, None, None),
            damage_entry("sting", "sting", s, 0.1, None, None),
            damage_entry("sweet_berry_bush", "sweetBerryBush", s, 0.1, Some("poking"), None),
            damage_entry("thorns", "thorns", s, 0.1, Some("thorns"), None),
            damage_entry("thrown", "thrown", s, 0.1, None, None),
            damage_entry("trident", "trident", s, 0.1, None, None),
            damage_entry("unattributed_fireball", "onFire", s, 0.1, Some("burning"), None),
            damage_entry("wind_charge", "mob", s, 0.1, None, None),
            damage_entry("wither", "wither", s, 0.0, None, None),
            damage_entry("wither_skull", "witherSkull", s, 0.1, None, None),
        ],
    }
}

fn build_trim_pattern_registry() -> InternalPacket {
    InternalPacket::RegistryData {
        registry_id: "minecraft:trim_pattern".into(),
        entries: vec![],
    }
}

fn build_trim_material_registry() -> InternalPacket {
    InternalPacket::RegistryData {
        registry_id: "minecraft:trim_material".into(),
        entries: vec![],
    }
}

fn build_wolf_variant_registry() -> InternalPacket {
    InternalPacket::RegistryData {
        registry_id: "minecraft:wolf_variant".into(),
        entries: vec![RegistryEntry {
            id: "minecraft:pale".into(),
            data: Some(nbt_compound! {
                "wild_texture" => NbtValue::String("minecraft:textures/entity/wolf/wolf.png".into()),
                "tame_texture" => NbtValue::String("minecraft:textures/entity/wolf/wolf_tame.png".into()),
                "angry_texture" => NbtValue::String("minecraft:textures/entity/wolf/wolf_angry.png".into()),
                "biomes" => NbtValue::String("minecraft:plains".into())
            }),
        }],
    }
}

fn build_painting_variant_registry() -> InternalPacket {
    InternalPacket::RegistryData {
        registry_id: "minecraft:painting_variant".into(),
        entries: vec![RegistryEntry {
            id: "minecraft:kebab".into(),
            data: Some(nbt_compound! {
                "asset_id" => NbtValue::String("minecraft:kebab".into()),
                "width" => NbtValue::Int(1),
                "height" => NbtValue::Int(1)
            }),
        }],
    }
}

fn build_banner_pattern_registry() -> InternalPacket {
    InternalPacket::RegistryData {
        registry_id: "minecraft:banner_pattern".into(),
        entries: vec![],
    }
}

fn enchant_entry(
    id: &str, description: &str, supported_items: &str,
    weight: i32, max_level: i32,
    min_base: i32, min_per_level: i32,
    max_base: i32, max_per_level: i32,
    anvil_cost: i32, slots: &[&str],
) -> RegistryEntry {
    let slot_list: Vec<NbtValue> = slots.iter().map(|s| NbtValue::String(s.to_string())).collect();
    RegistryEntry {
        id: format!("minecraft:{}", id),
        data: Some(nbt_compound! {
            "description" => nbt_compound! { "translate" => NbtValue::String(format!("enchantment.minecraft.{}", id)) },
            "supported_items" => NbtValue::String(supported_items.into()),
            "weight" => NbtValue::Int(weight),
            "max_level" => NbtValue::Int(max_level),
            "min_cost" => nbt_compound! {
                "base" => NbtValue::Int(min_base),
                "per_level_above_first" => NbtValue::Int(min_per_level)
            },
            "max_cost" => nbt_compound! {
                "base" => NbtValue::Int(max_base),
                "per_level_above_first" => NbtValue::Int(max_per_level)
            },
            "anvil_cost" => NbtValue::Int(anvil_cost),
            "slots" => NbtValue::List(slot_list)
        }),
    }
}

fn build_enchantment_registry() -> InternalPacket {
    InternalPacket::RegistryData {
        registry_id: "minecraft:enchantment".into(),
        entries: vec![
            // Armor enchantments
            enchant_entry("protection", "Protection", "#minecraft:enchantable/armor", 10, 4, 1, 11, 12, 11, 1, &["armor"]),
            enchant_entry("fire_protection", "Fire Protection", "#minecraft:enchantable/armor", 5, 4, 10, 8, 18, 8, 1, &["armor"]),
            enchant_entry("feather_falling", "Feather Falling", "#minecraft:enchantable/armor", 5, 4, 5, 6, 11, 6, 1, &["feet"]),
            enchant_entry("blast_protection", "Blast Protection", "#minecraft:enchantable/armor", 2, 4, 5, 8, 13, 8, 2, &["armor"]),
            enchant_entry("projectile_protection", "Projectile Protection", "#minecraft:enchantable/armor", 5, 4, 3, 6, 9, 6, 1, &["armor"]),
            enchant_entry("respiration", "Respiration", "#minecraft:enchantable/armor", 2, 3, 10, 10, 40, 10, 2, &["head"]),
            enchant_entry("aqua_affinity", "Aqua Affinity", "#minecraft:enchantable/armor", 2, 1, 1, 0, 41, 0, 2, &["head"]),
            enchant_entry("thorns", "Thorns", "#minecraft:enchantable/armor", 1, 3, 10, 20, 60, 20, 4, &["chest"]),
            enchant_entry("depth_strider", "Depth Strider", "#minecraft:enchantable/armor", 2, 3, 10, 10, 25, 10, 2, &["feet"]),
            enchant_entry("frost_walker", "Frost Walker", "#minecraft:enchantable/armor", 2, 2, 10, 10, 25, 10, 2, &["feet"]),
            enchant_entry("binding_curse", "Curse of Binding", "#minecraft:enchantable/armor", 1, 1, 25, 0, 50, 0, 4, &["armor"]),
            enchant_entry("soul_speed", "Soul Speed", "#minecraft:enchantable/armor", 1, 3, 10, 10, 25, 10, 4, &["feet"]),
            enchant_entry("swift_sneak", "Swift Sneak", "#minecraft:enchantable/armor", 1, 3, 25, 25, 75, 25, 4, &["legs"]),
            // Weapon enchantments
            enchant_entry("sharpness", "Sharpness", "#minecraft:enchantable/sword", 10, 5, 1, 11, 21, 11, 1, &["mainhand"]),
            enchant_entry("smite", "Smite", "#minecraft:enchantable/sword", 5, 5, 5, 8, 25, 8, 1, &["mainhand"]),
            enchant_entry("bane_of_arthropods", "Bane of Arthropods", "#minecraft:enchantable/sword", 5, 5, 5, 8, 25, 8, 1, &["mainhand"]),
            enchant_entry("knockback", "Knockback", "#minecraft:enchantable/sword", 5, 2, 5, 20, 55, 20, 1, &["mainhand"]),
            enchant_entry("fire_aspect", "Fire Aspect", "#minecraft:enchantable/sword", 2, 2, 10, 20, 60, 20, 2, &["mainhand"]),
            enchant_entry("looting", "Looting", "#minecraft:enchantable/sword", 2, 3, 15, 9, 65, 9, 2, &["mainhand"]),
            enchant_entry("sweeping_edge", "Sweeping Edge", "#minecraft:enchantable/sword", 2, 3, 5, 9, 20, 9, 2, &["mainhand"]),
            // Tool enchantments
            enchant_entry("efficiency", "Efficiency", "#minecraft:enchantable/mining", 10, 5, 1, 10, 51, 10, 1, &["mainhand"]),
            enchant_entry("silk_touch", "Silk Touch", "#minecraft:enchantable/mining", 1, 1, 15, 0, 65, 0, 4, &["mainhand"]),
            enchant_entry("unbreaking", "Unbreaking", "#minecraft:enchantable/durability", 5, 3, 5, 8, 55, 8, 1, &["any"]),
            enchant_entry("fortune", "Fortune", "#minecraft:enchantable/mining", 2, 3, 15, 9, 65, 9, 2, &["mainhand"]),
            enchant_entry("power", "Power", "#minecraft:enchantable/bow", 10, 5, 1, 10, 16, 10, 1, &["mainhand"]),
            enchant_entry("punch", "Punch", "#minecraft:enchantable/bow", 2, 2, 12, 20, 37, 20, 2, &["mainhand"]),
            enchant_entry("flame", "Flame", "#minecraft:enchantable/bow", 2, 1, 20, 0, 50, 0, 2, &["mainhand"]),
            enchant_entry("infinity", "Infinity", "#minecraft:enchantable/bow", 1, 1, 20, 0, 50, 0, 4, &["mainhand"]),
            enchant_entry("luck_of_the_sea", "Luck of the Sea", "#minecraft:enchantable/fishing", 2, 3, 15, 9, 65, 9, 2, &["mainhand"]),
            enchant_entry("lure", "Lure", "#minecraft:enchantable/fishing", 2, 3, 15, 9, 65, 9, 2, &["mainhand"]),
            enchant_entry("loyalty", "Loyalty", "#minecraft:enchantable/trident", 5, 3, 12, 7, 50, 7, 1, &["mainhand"]),
            enchant_entry("impaling", "Impaling", "#minecraft:enchantable/trident", 2, 5, 1, 8, 21, 8, 2, &["mainhand"]),
            enchant_entry("riptide", "Riptide", "#minecraft:enchantable/trident", 2, 3, 17, 7, 50, 7, 2, &["mainhand"]),
            enchant_entry("channeling", "Channeling", "#minecraft:enchantable/trident", 1, 1, 25, 0, 50, 0, 4, &["mainhand"]),
            enchant_entry("multishot", "Multishot", "#minecraft:enchantable/crossbow", 2, 1, 20, 0, 50, 0, 2, &["mainhand"]),
            enchant_entry("quick_charge", "Quick Charge", "#minecraft:enchantable/crossbow", 5, 3, 12, 20, 50, 20, 1, &["mainhand"]),
            enchant_entry("piercing", "Piercing", "#minecraft:enchantable/crossbow", 10, 4, 1, 10, 50, 10, 1, &["mainhand"]),
            // Special
            enchant_entry("mending", "Mending", "#minecraft:enchantable/durability", 2, 1, 25, 0, 75, 0, 2, &["any"]),
            enchant_entry("vanishing_curse", "Curse of Vanishing", "#minecraft:enchantable/vanishing", 1, 1, 25, 0, 50, 0, 4, &["any"]),
            // 1.21 additions
            enchant_entry("density", "Density", "#minecraft:enchantable/mace", 5, 5, 1, 11, 21, 11, 1, &["mainhand"]),
            enchant_entry("breach", "Breach", "#minecraft:enchantable/mace", 2, 4, 15, 9, 65, 9, 2, &["mainhand"]),
            enchant_entry("wind_burst", "Wind Burst", "#minecraft:enchantable/mace", 2, 3, 15, 9, 65, 9, 4, &["mainhand"]),
        ],
    }
}

fn build_jukebox_song_registry() -> InternalPacket {
    InternalPacket::RegistryData {
        registry_id: "minecraft:jukebox_song".into(),
        entries: vec![],
    }
}
