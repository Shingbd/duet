# Duet Workbench
Independent Rust project. Do not change the adjacent agent-workspace repository.
Read README.md and docs/phase-0.md before changes.
Keep application state independent of terminal rendering and Pi protocol.
A review always refers to an immutable Git snapshot, never a mutable file.
Do not treat agent_end, a summary, or a passing test as human approval.
Default offline demo must not call a model or change an external repository.
No automatic merging, deployment, credential logging, or loading arbitrary Pi extensions.
Run cargo fmt --check, cargo clippy --all-targets -- -D warnings, and cargo test.
