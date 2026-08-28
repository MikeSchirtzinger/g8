# Intent

The scorer/ directory implements the relevance scoring pipeline.
It is the load-bearing seam for the ranking system.

# Owner

agent: scorer-specialist
team: core-ranker
contact: #checkout-api-core

# Parked Ideas

- config-hot-reload: requires Arc<RwLock<Config>> across tokio tasks
- ml-scoring: requires Python FFI

# Decisions

ScoringStrategy is a sync trait to enable deterministic unit testing.
