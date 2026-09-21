use crate::{SolverConfig, World};
use pcb_core::Frame;

pub trait Backend {
    fn name(&self) -> &'static str;
    fn step(&mut self, world: &mut World, config: &SolverConfig, step: u64) -> Frame;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GpuKernel {
    ScatterField,
    SmoothPressure,
    DifferentiatePressure,
    GatherField,
    AccumulateTraceTension,
    Integrate,
    BuildBroadPhase,
    GenerateContacts,
    ProjectEqualityDistance,
    ProjectMaximumDistance,
    ProjectSegmentClearance,
    ProjectSegmentBodyClearance,
    ProjectBodyBodyClearance,
    ProjectAttachments,
    ClampBounds,
    ReduceMetrics,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpuDispatchPlan {
    pub kernels: Vec<GpuKernel>,
}

impl Default for GpuDispatchPlan {
    fn default() -> Self {
        Self {
            kernels: vec![
                GpuKernel::ScatterField,
                GpuKernel::SmoothPressure,
                GpuKernel::DifferentiatePressure,
                GpuKernel::GatherField,
                GpuKernel::AccumulateTraceTension,
                GpuKernel::Integrate,
                GpuKernel::BuildBroadPhase,
                GpuKernel::GenerateContacts,
                GpuKernel::ProjectEqualityDistance,
                GpuKernel::ProjectMaximumDistance,
                GpuKernel::ProjectSegmentClearance,
                GpuKernel::ProjectSegmentBodyClearance,
                GpuKernel::ProjectBodyBodyClearance,
                GpuKernel::ProjectAttachments,
                GpuKernel::ClampBounds,
                GpuKernel::ReduceMetrics,
            ],
        }
    }
}
