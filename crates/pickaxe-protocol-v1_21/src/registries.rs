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
                "fixed_time" => NbtValue::Long(-1), // not present = no fixed time, but we use optional
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

fn build_damage_type_registry() -> InternalPacket {
    InternalPacket::RegistryData {
        registry_id: "minecraft:damage_type".into(),
        entries: vec![
            RegistryEntry {
                id: "minecraft:generic".into(),
                data: Some(nbt_compound! {
                    "message_id" => NbtValue::String("generic".into()),
                    "scaling" => NbtValue::String("never".into()),
                    "exhaustion" => NbtValue::Float(0.0)
                }),
            },
            RegistryEntry {
                id: "minecraft:generic_kill".into(),
                data: Some(nbt_compound! {
                    "message_id" => NbtValue::String("genericKill".into()),
                    "scaling" => NbtValue::String("never".into()),
                    "exhaustion" => NbtValue::Float(0.0)
                }),
            },
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

fn build_enchantment_registry() -> InternalPacket {
    InternalPacket::RegistryData {
        registry_id: "minecraft:enchantment".into(),
        entries: vec![],
    }
}

fn build_jukebox_song_registry() -> InternalPacket {
    InternalPacket::RegistryData {
        registry_id: "minecraft:jukebox_song".into(),
        entries: vec![],
    }
}
