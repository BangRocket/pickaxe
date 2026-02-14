use pickaxe_types::{GameProfile, Vec3d};

/// Per-connection player state tracked on the network thread.
pub struct PlayerHandle {
    pub entity_id: i32,
    pub profile: GameProfile,
    pub position: Vec3d,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub view_distance: i32,
}

impl PlayerHandle {
    pub fn new(
        entity_id: i32,
        profile: GameProfile,
        position: Vec3d,
        yaw: f32,
        pitch: f32,
        chunk_x: i32,
        chunk_z: i32,
        view_distance: i32,
    ) -> Self {
        Self {
            entity_id,
            profile,
            position,
            yaw,
            pitch,
            on_ground: true,
            chunk_x,
            chunk_z,
            view_distance,
        }
    }

    pub fn update_position(&mut self, x: f64, y: f64, z: f64, on_ground: bool) {
        self.position = Vec3d::new(x, y, z);
        self.on_ground = on_ground;
    }

    pub fn update_position_and_rotation(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        yaw: f32,
        pitch: f32,
        on_ground: bool,
    ) {
        self.position = Vec3d::new(x, y, z);
        self.yaw = yaw;
        self.pitch = pitch;
        self.on_ground = on_ground;
    }

    pub fn update_rotation(&mut self, yaw: f32, pitch: f32, on_ground: bool) {
        self.yaw = yaw;
        self.pitch = pitch;
        self.on_ground = on_ground;
    }
}
