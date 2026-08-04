# Agentic Development Kit

- 日本語で簡潔かつ丁寧に回答する。
- Rust実装はRepository直下にある。テスト資材は `testdata/`、補助scriptは `scripts/`、設計記録は `docs/` に置く。
- 変更後は `cargo fmt --check`、`cargo clippy --all-targets --locked -- -D warnings`、`cargo test --locked`、`sh scripts/tests/test-vnext-rust.sh` を実行する。
- `project init` が配置するのは `skill-src/` のSkillと `templates/` の2ファイルだけ。ここへ追加するものは、現在のRecord形式で読み込めることをテストで固定する。
- 既存プロジェクトへの導入では、利用者のファイルを上書きしない。`AGENTS.md` への追記だけが例外で、管理ブロックが既にあれば何もしない。
- CLI、Schema、Skill、説明書、テストは同時に更新する。説明書は英語の`README.md`が正で、`docs/concepts.ja.md`が日本語の解説。
- 意味的な設計判断を自動化しない。既存コードの分析と契約の記入はエージェントの責務とする。
- `contracts/` は現在有効な規範、`decisions/` は判断履歴、`docs/` は利用案内と索引に限定する。
