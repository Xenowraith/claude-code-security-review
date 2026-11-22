//! SDK IMU ownership. Orientation/motion getters claim full IMU; stop releases
//! that claim. Apps can opt in to thresholds alongside full IMU sampling.
#[link(wasm_import_module = "mira")]
extern "C" {
    fn imu_threshold(above_cdeg:i32,below_cdeg:i32)->i32;
    fn imu_stop();
    fn imu_share_thresholds(enabled:i32)->i32;
}

// The look-up threshold was lowered by three degrees; hysteresis is three degrees.
pub const LOOK_UP_CDEG: i32 = 2_400;
pub const LOOK_DOWN_CDEG: i32 = 2_100;

/// 0 below, 1 above, -1 asleep/unavailable, -2 exclusive full IMU requested.
pub unsafe fn threshold_state()->i32 { imu_threshold(LOOK_UP_CDEG, LOOK_DOWN_CDEG) }

/// Keeps the resident's look-up gesture available alongside this app's IMU use.
/// The policy lasts until changed or the app exits, including across `stop()`.
#[allow(dead_code)]
pub unsafe fn share_thresholds(enabled:bool) { imu_share_thresholds(enabled as i32); }

#[allow(dead_code)]
pub unsafe fn stop() { imu_stop(); }
