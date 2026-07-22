# 導入レベル

現在の規模だけでなく、失敗時の影響と変更の不可逆性で選ぶ。上位レベルは下位レベルを含む。

## Lite

個人開発、短期プロトタイプ、捨てられる検証に使う。

- Project ContractとFeature Contract
- Contract decision authorityと一時的なDecision Request
- 実装前Contract / Authority Challenge
- Decisionによる判断履歴
- 最小のユニットテスト

## Standard

通常の業務アプリ、少人数チーム、継続運用するプロダクトに使う。迷った場合はこれを選ぶ。

- Lite の全項目
- Domain、Capability、Architecture Contract
- Platform 能力の検証台帳
- Data InvariantとOperation Contract
- データ関係、多重度、所有権、transaction境界
- 境界に対する contract test と適合性テスト

## System

複数サービス、複数チーム、複数エージェント、長期運用、外部基盤との複雑な統合に使う。

- Standard の全項目
- Contract resolverとresolved lock
- Mutation Graphとactive change競合検出
- Bounded Context、非同期、外部同期、Frontendの共通Contract
- 実環境に近い platform probe
- Stateful Challengerと操作列・並行作業の検証

## Critical

金銭、機密情報、権限、法令、監査、不可逆な削除や公開を伴う変更に使う。

- System の全項目
- Threat Model、Failure Model、runtime invariant
- リリース証拠と承認対象の明示
- Failure Injection、reconciliation、復旧確認
- Incident Learning

## 一時的な昇格条件

プロジェクト全体を昇格させなくても、次の変更は一段上の証拠を要求する。

- 保存形式や多重度を変える
- 認証・認可・テナント境界を変える
- 非同期化、SSE、外部同期などプロトコルを変える
- データ削除、課金、公開など不可逆な作用を持つ
- SDK やマネージドサービスの未検証能力に依存する
- 複数の実装が同じ業務能力を別ルールで表現している
