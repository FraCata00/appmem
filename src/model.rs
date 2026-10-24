//! Data types shared by the collector, the grouping logic and the UI.

use std::path::PathBuf;

/// One process as read from the system. Fields that could not be read are `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcInfo {
    pub pid: i32,
    pub ppid: i32,
    pub uid: u32,
    pub name: String,
    pub path: Option<PathBuf>,
    /// Physical footprint in bytes, the value Activity Monitor shows as "Memory".
    pub footprint: Option<u64>,
    pub responsible_pid: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressure {
    Normal,
    Warn,
    Critical,
}

impl Pressure {
    /// Maps `kern.memorystatus_vm_pressure_level` (1, 2, 4) to a level.
    pub fn from_level(level: i32) -> Pressure {
        match level {
            2 => Pressure::Warn,
            4 => Pressure::Critical,
            _ => Pressure::Normal,
        }
    }
}

/// System-wide memory numbers, all in bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemStats {
    pub total: u64,
    pub used: u64,
    pub wired: u64,
    pub swap_used: u64,
    /// App memory with compressed pages at their full size: the space process
    /// footprints are measured in, used to estimate what couldn't be read.
    pub app_footprint: u64,
    pub pressure: Pressure,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub procs: Vec<ProcInfo>,
    pub stats: SystemStats,
    pub current_uid: u32,
    pub self_pid: i32,
}
