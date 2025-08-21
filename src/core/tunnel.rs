use crate::scheduler::TunnelScheduler;
use crate::progress::ProgressAggregator;

pub struct Tunnel {
    scheduler: TunnelScheduler,
    aggregator: Option<ProgressAggregator>,
}

impl Tunnel {
    pub fn new() -> Self {
        Self {
            aggregator: None,
            scheduler: TunnelScheduler::new(),
        }
    }

    pub fn with_aggregator(mut self, aggregator: ProgressAggregator) -> Self {
        self.aggregator = Some(aggregator);
        self
    }
}


