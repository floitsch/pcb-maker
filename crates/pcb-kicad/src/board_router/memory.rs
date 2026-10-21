// Copyright (C) 2026 Toit contributors.

//! Memory measures that decide how many routers run side by side, and the
//! limits they are measured against.
//!
//! A router's peak is estimated from its lattice maps (the router's own
//! `memory: N MB in maps` line), a measured peak-to-maps ratio and what
//! a router holds beside its maps, with a safety factor; the routers that run at once are as many as fit the
//! tightest of the process's limits: its address space (RLIMIT_AS), its
//! cgroup's memory.max (v2, every level up to the root, or v1's
//! memory.limit_in_bytes), an explicit limit (`PCB_MAKER_MEMORY_MB` or the
//! router setting `memory_mb`) and, shared with the rest of the machine,
//! half of MemAvailable. Until 2026-10-21 only MemAvailable counted, so a
//! harness's cap went unseen: 456 runs of the 5 834-board intake died at a
//! 3 GiB cap with two to four routers side by side.

use super::*;

/// A router's peak resident size from its lattice maps (`memory: N MB in
/// maps`): `PEAK_RESIDENT_PER_MAPS` times the maps and
/// `RESIDENT_OVERHEAD_MB` beside them (the board parsed and lowered, its
/// pours, the router's own records). Measured 2026-10-21 with one router
/// at a time and one search thread (the whole process's peak), on five
/// intake boards and Castor, jetson, k30: peak over maps 1.05-1.68 where
/// the maps are 240-760 MB, 2.5-3.2 on the three v16 boards (about 300 MB
/// of maps and up to 570 MB beside them); the line covers every board
/// (docs/changelog.md, "ladder memory and work bound").
pub(crate) const PEAK_RESIDENT_PER_MAPS: f64 = 1.7;
pub(crate) const RESIDENT_OVERHEAD_MB: f64 = 460.0;

/// The same for the address space (VmPeak): reserved but untouched pages,
/// thread stacks and malloc's arenas count too (peak over maps 1.41-3.8 on
/// the same boards).
pub(crate) const PEAK_ADDRESS_SPACE_PER_MAPS: f64 = 2.0;
pub(crate) const ADDRESS_SPACE_OVERHEAD_MB: f64 = 550.0;

/// The margin on a router's estimated peak: another board's lies off the
/// measured line, a finer lattice's off its regular one's.
pub(crate) const MEMORY_SAFETY: f64 = 1.25;

/// The share of the machine's available memory routers side by side may
/// take: other work on the machine grows too.
pub(crate) const AVAILABLE_SHARE: f64 = 0.5;

/// The process's resident size now, in megabytes (Linux; elsewhere 0).
pub(crate) fn resident_mb() -> f64 {
    status_field("VmRSS:").unwrap_or(0.0)
}

/// A field of /proc/self/status in megabytes (Linux; elsewhere None).
fn status_field(name: &str) -> Option<f64> {
    kilobytes_field(&fs::read_to_string("/proc/self/status").ok()?, name)
}

/// The value of a `Name: N kB` line of a /proc file, in megabytes.
fn kilobytes_field(text: &str, name: &str) -> Option<f64> {
    text.lines()
        .find(|line| line.starts_with(name))?
        .split_whitespace()
        .nth(1)?
        .parse::<f64>()
        .ok()
        .map(|kb| kb / 1024.0)
}

/// The machine's memory, in megabytes: a fixed share of it decides what
/// the ladder may attempt, so that the attempts do not depend on what else
/// runs (the memory available decides only how many run at once).
pub(crate) fn memory_total_mb() -> Option<f64> {
    kilobytes_field(&fs::read_to_string("/proc/meminfo").ok()?, "MemTotal:")
}

/// The process's peak resident size and the memory free for it, in
/// megabytes (Linux; elsewhere unknown): the machine's MemAvailable, or
/// less under a limit of the process's own (`MemoryLimits`).
pub(crate) fn memory_mb() -> Option<(f64, f64)> {
    let peak = status_field("VmHWM:")?;
    let limits = MemoryLimits::read(None);
    let available = limits.machine_available()?;
    Some((
        peak,
        limits
            .hard_free()
            .map_or(available, |free| free.min(available)),
    ))
}

/// What a limit bounds: the address space (virtual size, reserved but
/// untouched pages included) or resident memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemoryKind {
    AddressSpace,
    Resident,
}

/// One memory limit of the process and what is free under it now.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MemoryLimit {
    /// Where it comes from, for the log.
    pub source: &'static str,
    pub kind: MemoryKind,
    /// The limit, in megabytes (MemTotal for the machine's).
    pub limit_mb: f64,
    /// What is free under it now, in megabytes.
    pub free_mb: f64,
    /// Shared with the rest of the machine (MemAvailable): routers side by
    /// side take only `AVAILABLE_SHARE` of it. A limit of the process's
    /// own is all its.
    pub shared: bool,
}

impl MemoryLimit {
    /// What routers side by side may take of it.
    pub(crate) fn usable_mb(&self) -> f64 {
        if self.shared {
            self.free_mb * AVAILABLE_SHARE
        } else {
            self.free_mb
        }
        .max(0.0)
    }

    /// A router's estimated peak under this limit, from its maps.
    pub(crate) fn router_mb(&self, maps_mb: f64) -> f64 {
        router_peak_mb(maps_mb, self.kind)
    }
}

/// A router's estimated peak, from its lattice maps, as `kind` counts it.
pub(crate) fn router_peak_mb(maps_mb: f64, kind: MemoryKind) -> f64 {
    let (ratio, overhead) = match kind {
        MemoryKind::AddressSpace => (PEAK_ADDRESS_SPACE_PER_MAPS, ADDRESS_SPACE_OVERHEAD_MB),
        MemoryKind::Resident => (PEAK_RESIDENT_PER_MAPS, RESIDENT_OVERHEAD_MB),
    };
    (maps_mb * ratio + overhead) * MEMORY_SAFETY
}

/// What the limit resolution reads, gathered once (`MemoryInputs::read`),
/// so that the resolution (`MemoryLimits::resolve`) is a function of it.
#[derive(Clone, Debug, Default)]
pub(crate) struct MemoryInputs {
    /// RLIMIT_AS (the soft limit), when not unlimited.
    pub address_space_mb: Option<f64>,
    /// The process's virtual size (VmSize) and resident size (VmRSS).
    pub virtual_mb: f64,
    pub resident_mb: f64,
    /// Every cgroup v2 level with a memory.max: (memory.max, its memory
    /// in use: memory.current less the reclaimable inactive_file).
    pub cgroup_v2: Vec<(f64, f64)>,
    /// cgroup v1's memory.limit_in_bytes and usage, when limited.
    pub cgroup_v1: Option<(f64, f64)>,
    /// `PCB_MAKER_MEMORY_MB` and the router setting `memory_mb`.
    pub environment_mb: Option<f64>,
    pub setting_mb: Option<f64>,
    /// MemTotal and MemAvailable.
    pub total_mb: Option<f64>,
    pub available_mb: Option<f64>,
    /// The static maps and the connected pours' masks the process holds
    /// (pcb-router's `held_maps_bytes`: its routers' and the ladder's
    /// holds), in the resident size above.
    pub held_statics_mb: f64,
    pub held_masks_mb: f64,
}

const BYTES_PER_MB: f64 = 1024.0 * 1024.0;

/// A cgroup file's number of bytes in megabytes; None for `max`, an
/// unreadable file or v1's "unlimited" (a page-rounded 2^63).
fn cgroup_bytes(path: &Path) -> Option<f64> {
    let text = fs::read_to_string(path).ok()?;
    let value = text.trim().parse::<u64>().ok()?;
    (value < 1 << 60).then_some(value as f64 / BYTES_PER_MB)
}

/// A `name value` line of a cgroup's memory.stat, in megabytes.
fn cgroup_stat(directory: &Path, name: &str) -> f64 {
    fs::read_to_string(directory.join("memory.stat"))
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                line.strip_prefix(name)?
                    .strip_prefix(' ')?
                    .trim()
                    .parse::<u64>()
                    .ok()
            })
        })
        .map_or(0.0, |bytes| bytes as f64 / BYTES_PER_MB)
}

/// The process's cgroup paths from /proc/self/cgroup: v2's (`0::/path`)
/// and v1's memory controller's (`N:memory:/path` or with other
/// controllers beside it).
pub(crate) fn cgroup_paths(text: &str) -> (Option<String>, Option<String>) {
    let mut v2 = None;
    let mut v1 = None;
    for line in text.lines() {
        let mut parts = line.splitn(3, ':');
        let (Some(id), Some(controllers), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if id == "0" && controllers.is_empty() {
            v2 = Some(path.to_string());
        } else if controllers
            .split(',')
            .any(|controller| controller == "memory")
        {
            v1 = Some(path.to_string());
        }
    }
    (v2, v1)
}

impl MemoryInputs {
    /// Reads the process's limits and the machine's memory (Linux; empty
    /// elsewhere), with the router setting `setting_mb`.
    pub(crate) fn read(setting_mb: Option<f64>) -> Self {
        let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
        let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let mut inputs = Self {
            address_space_mb: address_space_limit_mb(),
            virtual_mb: kilobytes_field(&status, "VmSize:").unwrap_or(0.0),
            resident_mb: kilobytes_field(&status, "VmRSS:").unwrap_or(0.0),
            environment_mb: std::env::var("PCB_MAKER_MEMORY_MB")
                .ok()
                .and_then(|value| value.trim().parse::<f64>().ok())
                .filter(|megabytes| *megabytes > 0.0),
            setting_mb: setting_mb.filter(|megabytes| *megabytes > 0.0),
            total_mb: kilobytes_field(&meminfo, "MemTotal:"),
            available_mb: kilobytes_field(&meminfo, "MemAvailable:"),
            ..Self::default()
        };
        let (statics, masks) = pcb_router::router::held_maps_bytes();
        inputs.held_statics_mb = statics as f64 / BYTES_PER_MB;
        inputs.held_masks_mb = masks as f64 / BYTES_PER_MB;
        let (v2, v1) = cgroup_paths(&fs::read_to_string("/proc/self/cgroup").unwrap_or_default());
        if let Some(path) = v2 {
            // Every level's memory.max holds, the root's included.
            let root = Path::new("/sys/fs/cgroup");
            let mut directory = root.join(path.trim_start_matches('/'));
            loop {
                if let Some(max) = cgroup_bytes(&directory.join("memory.max")) {
                    let current = cgroup_bytes(&directory.join("memory.current")).unwrap_or(0.0);
                    let used = (current - cgroup_stat(&directory, "inactive_file")).max(0.0);
                    inputs.cgroup_v2.push((max, used));
                }
                if directory == root || !directory.pop() || !directory.starts_with(root) {
                    break;
                }
            }
        }
        if let Some(path) = v1 {
            let directory = Path::new("/sys/fs/cgroup/memory").join(path.trim_start_matches('/'));
            if let Some(limit) = cgroup_bytes(&directory.join("memory.limit_in_bytes")) {
                let usage = cgroup_bytes(&directory.join("memory.usage_in_bytes")).unwrap_or(0.0);
                let used = (usage - cgroup_stat(&directory, "total_inactive_file")).max(0.0);
                inputs.cgroup_v1 = Some((limit, used));
            }
        }
        inputs
    }
}

/// RLIMIT_AS's soft limit in megabytes, when there is one.
fn address_space_limit_mb() -> Option<f64> {
    #[cfg(unix)]
    {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: getrlimit writes the struct it is given and nothing else.
        let read = unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut limit) } == 0;
        (read && limit.rlim_cur != libc::RLIM_INFINITY)
            .then_some(limit.rlim_cur as f64 / BYTES_PER_MB)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// The process's memory limits as the ladder sizes its routers by them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MemoryLimits {
    pub limits: Vec<MemoryLimit>,
    /// The static maps and pour masks held in the process as the limits
    /// were read (`MemoryInputs::held_statics_mb`), named in `describe`:
    /// they are in its resident size, so what is free under each limit
    /// counts them; the ladder holds a phase's maps (`Ladder::new_phase`)
    /// and releases them when the phase ends.
    pub held_statics_mb: f64,
    pub held_masks_mb: f64,
}

impl MemoryLimits {
    /// Reads the limits now (`MemoryInputs::read`).
    pub(crate) fn read(setting_mb: Option<f64>) -> Self {
        Self::resolve(&MemoryInputs::read(setting_mb))
    }

    /// The limits `inputs` give: the address space, every cgroup level,
    /// the explicit limit (the smaller of the environment's and the
    /// setting's, against the resident size) and the machine's available
    /// memory (shared).
    pub(crate) fn resolve(inputs: &MemoryInputs) -> Self {
        let mut limits = Vec::new();
        if let Some(limit) = inputs.address_space_mb {
            limits.push(MemoryLimit {
                source: "the address space limit (RLIMIT_AS)",
                kind: MemoryKind::AddressSpace,
                limit_mb: limit,
                free_mb: limit - inputs.virtual_mb,
                shared: false,
            });
        }
        if let Some((limit, used)) = inputs
            .cgroup_v2
            .iter()
            .copied()
            .min_by(|a, b| (a.0 - a.1).total_cmp(&(b.0 - b.1)))
        {
            limits.push(MemoryLimit {
                source: "the cgroup's memory.max",
                kind: MemoryKind::Resident,
                limit_mb: limit,
                free_mb: limit - used,
                shared: false,
            });
        }
        if let Some((limit, used)) = inputs.cgroup_v1 {
            limits.push(MemoryLimit {
                source: "the cgroup's memory.limit_in_bytes (v1)",
                kind: MemoryKind::Resident,
                limit_mb: limit,
                free_mb: limit - used,
                shared: false,
            });
        }
        let explicit = match (inputs.environment_mb, inputs.setting_mb) {
            (Some(environment), Some(setting)) if setting < environment => {
                Some((setting, "the router setting memory_mb"))
            }
            (Some(environment), _) => Some((environment, "PCB_MAKER_MEMORY_MB")),
            (None, Some(setting)) => Some((setting, "the router setting memory_mb")),
            (None, None) => None,
        };
        if let Some((limit, source)) = explicit {
            limits.push(MemoryLimit {
                source,
                kind: MemoryKind::Resident,
                limit_mb: limit,
                free_mb: limit - inputs.resident_mb,
                shared: false,
            });
        }
        if let Some(available) = inputs.available_mb {
            limits.push(MemoryLimit {
                source: "the machine's available memory (MemAvailable)",
                kind: MemoryKind::Resident,
                limit_mb: inputs.total_mb.unwrap_or(available),
                free_mb: available,
                shared: true,
            });
        }
        Self {
            limits,
            held_statics_mb: inputs.held_statics_mb,
            held_masks_mb: inputs.held_masks_mb,
        }
    }

    /// The machine's available memory (MemAvailable).
    pub(crate) fn machine_available(&self) -> Option<f64> {
        self.limits
            .iter()
            .find(|limit| limit.shared)
            .map(|limit| limit.free_mb)
    }

    /// The least free under a limit of the process's own, if any.
    pub(crate) fn hard_free(&self) -> Option<f64> {
        self.limits
            .iter()
            .filter(|limit| !limit.shared)
            .map(|limit| limit.free_mb)
            .min_by(f64::total_cmp)
    }

    /// How many routers of `maps_mb` of maps each fit side by side, at
    /// least one and at most `wanted`, and the limit that decided it (the
    /// one that fits the fewest; None: no limit known, `wanted` run).
    pub(crate) fn routers_that_fit(
        &self,
        maps_mb: f64,
        wanted: usize,
    ) -> (usize, Option<&MemoryLimit>) {
        let wanted = wanted.max(1);
        let tightest = self
            .limits
            .iter()
            .map(|limit| {
                let each = limit.router_mb(maps_mb).max(1.0);
                ((limit.usable_mb() / each).floor().max(0.0) as usize, limit)
            })
            .min_by_key(|(count, _)| *count);
        match tightest {
            Some((count, limit)) => (count.clamp(1, wanted), Some(limit)),
            None => (wanted, None),
        }
    }

    /// The first limit of the process's own that `routers` routers of
    /// `maps_mb` of maps each do not fit at once, by the limit itself (a
    /// fixed number, so that what the ladder attempts does not depend on
    /// what else runs).
    pub(crate) fn too_small_for(&self, maps_mb: f64, routers: usize) -> Option<&MemoryLimit> {
        self.too_small_for_beside(maps_mb, routers, 0.0)
    }

    /// `too_small_for` with `beside_mb` held beside the routers (the rung
    /// probes' other static maps, `Ladder::probes`).
    pub(crate) fn too_small_for_beside(
        &self,
        maps_mb: f64,
        routers: usize,
        beside_mb: f64,
    ) -> Option<&MemoryLimit> {
        self.limits
            .iter()
            .filter(|limit| !limit.shared)
            .find(|limit| {
                routers as f64 * limit.router_mb(maps_mb) + beside_mb.max(0.0) > limit.limit_mb
            })
    }

    /// One line for the log: every limit, the tightest first.
    pub(crate) fn describe(&self) -> String {
        let mut limits: Vec<&MemoryLimit> = self.limits.iter().collect();
        limits.sort_by(|a, b| a.usable_mb().total_cmp(&b.usable_mb()));
        let held = if self.held_statics_mb + self.held_masks_mb >= 0.5 {
            format!(
                "; held: static maps {:.0} MB, pour masks {:.0} MB",
                self.held_statics_mb, self.held_masks_mb
            )
        } else {
            String::new()
        };
        if limits.is_empty() {
            return format!("no memory limit known{held}");
        }
        let described = limits
            .iter()
            .map(|limit| {
                format!(
                    "{} {:.0} MB, {:.0} MB free{}",
                    limit.source,
                    limit.limit_mb,
                    limit.free_mb,
                    if limit.shared {
                        format!(" ({:.0} % of it for routers)", AVAILABLE_SHARE * 100.0)
                    } else {
                        String::new()
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        format!("{described}{held}")
    }
}

/// How a side-by-side count was decided, for the log: the router's
/// estimated peak and the limit that held it.
pub(crate) fn fit_note(maps_mb: f64, limit: Option<&MemoryLimit>) -> String {
    match limit {
        Some(limit) => format!(
            "about {:.0} MB a router ({:.0} MB of maps), {:.0} MB usable under {}",
            limit.router_mb(maps_mb),
            maps_mb,
            limit.usable_mb(),
            limit.source
        ),
        None => "no memory limit known".into(),
    }
}

/// The megabytes of a router's static maps, from its memory report
/// (`...: statics N, ...`).
pub(crate) fn statics_mb_of(report: &str) -> Option<f64> {
    report
        .split_once("): statics ")?
        .1
        .split_once(',')?
        .0
        .trim()
        .parse::<f64>()
        .ok()
}

/// The limits as this process first read them for a lattice check
/// (`check_lattice_fits`), with the router setting `memory_mb` they were
/// read with: later lowerings of the run judge against the same numbers
/// (MemAvailable included), so that whether a board is refused does not
/// change halfway through a run, and read no files.
static LIMITS_AT_FIRST_CHECK: std::sync::Mutex<Option<(Option<u64>, MemoryLimits)>> =
    std::sync::Mutex::new(None);

/// The input's error when the router's lattice for `board` cannot be
/// held, checked before any lattice map is allocated: an outline the maps
/// cannot index (`LatticeSize::check`), or a router whose peak, estimated
/// from a floor on its maps at the coarsest pitch the router knows (the
/// ladder's own estimate: `PEAK_RESIDENT_PER_MAPS` times the maps and
/// `RESIDENT_OVERHEAD_MB`, without the safety margin), exceeds a limit
/// of the process's own (`PCB_MAKER_MEMORY_MB` or the router setting
/// `memory_mb`, the cgroup, the address space) or the machine's available
/// memory. Until 2026-10-21 one stray Edge.Cuts line made a lattice of
/// 10^10 nodes a map and the kernel killed the run without a verdict.
/// The verdict's next_actions name the extent and the pitch.
pub(crate) fn check_lattice_fits(
    board: &pcb_router::Board,
    config: &KiCadBoardRouterConfig,
) -> Result<(), String> {
    let pitch = config
        .grid_pitches_mm
        .iter()
        .flatten()
        .chain(pcb_router::router::Config::default().pitches.iter())
        .copied()
        .filter(|pitch| pitch.is_finite() && *pitch > 0.0)
        .fold(0.0, f64::max);
    let size = pcb_router::grid::LatticeSize::of(board, pitch);
    let refuse = |message: String| {
        crate::verdict::note_input_action(format!(
            "the board outline spans {:.1} x {:.1} mm: at the router's coarsest pitch of {pitch} mm that is {:.0} x {:.0} lattice nodes a layer; check Edge.Cuts for an item far from the board",
            size.extent[0], size.extent[1], size.nodes[0], size.nodes[1]
        ));
        Err(crate::verdict::input_error(message))
    };
    if let Err(message) = size.check() {
        return refuse(message);
    }
    let maps_mb = size.maps_bytes_floor(board.layer_count, board.classes.len()) / BYTES_PER_MB;
    let setting = config.memory_mb.map(f64::to_bits);
    let limits = {
        let mut first = LIMITS_AT_FIRST_CHECK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match &*first {
            Some((read_with, limits)) if *read_with == setting => limits.clone(),
            _ => {
                let limits = MemoryLimits::read(config.memory_mb);
                *first = Some((setting, limits.clone()));
                limits
            }
        }
    };
    for limit in &limits.limits {
        let router_mb = limit.router_mb(maps_mb) / MEMORY_SAFETY;
        let (allowed, what) = if limit.shared {
            (limit.free_mb, "available")
        } else {
            (limit.limit_mb, "allowed")
        };
        if router_mb > allowed {
            return refuse(format!(
                "the board outline spans {:.1} x {:.1} mm: at the router's coarsest pitch of {pitch} mm its lattice has {:.0} x {:.0} nodes on {} layers for {} rule classes, at least {maps_mb:.0} MB of maps, a router of at least {router_mb:.0} MB, more than the {allowed:.0} MB {} {what}",
                size.extent[0],
                size.extent[1],
                size.nodes[0],
                size.nodes[1],
                board.layer_count,
                board.classes.len(),
                limit.source
            ));
        }
    }
    Ok(())
}

/// The megabytes of a router's lattice maps, from its memory report
/// (`memory: N MB in maps (...)`).
pub(crate) fn maps_mb_of(report: &str) -> Option<f64> {
    report
        .strip_prefix("memory: ")?
        .split_once(" MB in maps")?
        .0
        .trim()
        .parse::<f64>()
        .ok()
}
