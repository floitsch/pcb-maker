use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StageReport {
    pub stage: String,
    pub changed: bool,
    pub work: u64,
    pub invalidated_analyses: Vec<String>,
    pub notes: Vec<String>,
}

pub trait Stage<Candidate> {
    fn name(&self) -> &'static str;
    fn run(&mut self, candidate: &mut Candidate) -> StageReport;
}

pub struct Pipeline<Candidate> {
    stages: Vec<Box<dyn Stage<Candidate>>>,
}

impl<Candidate> Pipeline<Candidate> {
    pub fn new() -> Self {
        Self { stages: Vec::new() }
    }

    pub fn push(&mut self, stage: impl Stage<Candidate> + 'static) {
        self.stages.push(Box::new(stage));
    }

    pub fn run(&mut self, candidate: &mut Candidate) -> Vec<StageReport> {
        self.stages
            .iter_mut()
            .map(|stage| stage.run(candidate))
            .collect()
    }
}

impl<Candidate> Default for Pipeline<Candidate> {
    fn default() -> Self {
        Self::new()
    }
}
