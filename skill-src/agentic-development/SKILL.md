---
name: agentic-development
description: Bootstrap or adopt the Agentic Development control plane in a new or existing repository. Use when selecting an assurance level, inventorying current sources of truth, recovering shared domain and architecture rules, identifying divergent implementations and platform unknowns, or migrating a repository to hierarchical contracts, readiness assessment, data invariants, operation contracts, and independent challenge.
---

# Agentic Development

## 目的

現在有効な正しさを階層Contractとして定着させ、変更開始前の準備判定と独立反証を可能にする。

## 開始手順

1. リポジトリ直下の `AGENTS.md` と `.agentic/installation.yaml` を読む。
2. `references/levels.md` を読み、現在のレベルがリスクに合うか確認する。
3. 既存プロジェクトへの導入では `references/adoption-workflow.md` を読む。
4. 対象リポジトリのテンプレートを調査結果に基づいて埋める。事実、推測、未確認を分離する。
5. 導入後の機能開発は `$agentic-change` から開始する。

## 基本ルール

- `contracts/`を現在有効な規範、`decisions/`を判断履歴、`docs/`を利用案内と索引に限定する。
- Feature Contractを独立仕様にしない。Project、Domain、Capability、Architecture、Data Invariant、Operation Contractを解決する。
- 類似機能を新しく設計する前に既存Contract、参照実装、共通能力を探す。異なる設計はDecisionとdeviationに残す。
- SDK、DB、検索、ストリーミング、認証などの外部境界は、名称や型注釈から能力を推測しない。最小 probe または公式仕様で検証する。
- Repository Observationで認可変更や機密データアクセスを扱う場合、method名から推測せず、Projectのデータ分類と認可境界をaccepted Decision付きBinding Recordへ明示する。
- 所有権、多重度、ライフサイクル、プロトコル、削除、互換性、アーキテクチャ境界をFeature内で暗黙に確定しない。
- resolve前のContract / Authority Challengerと、実装後のStateful Challengerを区別する。前者は要求、authority、decision、Contractの不足を探し、後者はlock、diff、証拠から契約違反、競合、部分失敗、再実行を探す。
- 不足や反例は仕様決定のauthorityにしない。既存authorityで決まらない選択は一時的なDecision Requestとして人へ提示し、確定後はDecisionとContractへ反映する。
- Builder が検証条件を弱めて成功扱いにしない。契約変更は契約側の判断として記録する。
- 完了は「コードがある」ではなく、契約、実装、証拠、残存リスクが揃った状態とする。

## 成果物

- 導入時: `docs/agentic/adoption-report.md` と `source-of-truth.md` を完成させる。
- 現在有効な正しさ: 適切なkindのContractへ昇格する。
- 判断理由: `decisions/`へ残し、Contractの現在値へ反映する。
- データ変更: Data Invariant、Operation Contract、Mutation Graphを整備する。
- 障害や手戻りの後: 個別修正だけで終えず、欠けていた契約、probe、規約、テストへ知識を昇格させる。

包括的レビューを必須ゲートにしない。高リスクな意味判断だけを人へ上げ、readiness、機械検査、独立反証を主な制御にする。
