1. small task
2. generate
3. run
4. debug
5. iterate

Start green, leave green. Before concluding any change:

```bash
cargo build && cargo test && cargo clippy --all-targets && cargo fmt
```
