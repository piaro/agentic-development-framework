# Agentic Development Kit

- 日本語で簡潔かつ丁寧に回答する。
- `bin/agentic-init` はPOSIX `sh` 互換を維持し、Bash固有構文を使わない。
- `bin/agentic` は Python 3.10 以上と PyYAML 6 系で動作させ、意味判断を自動化しない。
- 既存プロジェクトへの導入では、ユーザーファイルを上書きしない。`--upgrade`でもkit管理のSkill、CLI、AGENTS管理ブロックだけを更新する。
- Contract・レベル追加時は、CLI、Schema、README、Skill、テストを同時に更新する。
- Shellで意味的な設計判断を自動化しない。既存コード分析と契約記入はエージェントの責務とする。
- `contracts/` は現在有効な規範、`decisions/` は判断履歴、`docs/` は利用案内と索引に限定する。
- 変更後は `cargo fmt --check`、`cargo clippy --all-targets --locked -- -D warnings`、`cargo test --locked`、`sh scripts/tests/test-init.sh` を実行する。
- Rust実装はRepository直下にある。テスト資材は `testdata/`、補助scriptは `scripts/`、設計記録は `docs/` に置く。
