# 機能開発ワークフロー（互換案内）

新しい変更は`$agentic-change`から開始し、`$agentic-contract`、`$agentic-builder`、`$agentic-challenger`へ進む。本書は旧単一Skill利用時の原則を残す。

## 1. 意図を固定する

- ユーザーまたは業務上の成果
- 非対象
- 完了が確定する瞬間
- 失敗時に守るもの

を先に記述する。

## 2. リスクを分類する

- R0: 局所的、可逆、状態を持たない
- R1: 単一境界の通常変更
- R2: データ、外部サービス、並行処理、複数コンポーネントにまたがる
- R3: セキュリティ、金銭、不可逆操作、大規模移行、プロトコル変更

R1以上ではresolve前にContract / Authority Challengeを行う。R2以上では実装前・実装後とも独立 Challengerを置く。R3ではFailure Modelとリリース証拠も要求する。

## 3. 類似実装と共通能力を探す

名前だけでなく、同じ状態遷移、データ所有権、外部境界を持つ実装を探す。参照実装と異なる場合は、実装前に差異を説明する。

## 4. 上位Contractを解決してFeature Contractを作る

最低限、次を定義する。

- outcome と non-scope
- domain invariants と lifecycle
- 関係の多重度と所有権
- 入出力型と互換性
- 完了、失敗、取消、再試行の意味
- atomicity、idempotency、concurrency
- platform assumptions と probes
- conformance 先と許容する deviation
- 検証証拠と残存リスク

Feature Contractは上位Contractの差分とする。所有権、多重度、状態遷移、共有Protocol、保存形式などが未決定なら、実装を止めて上位Contractを先に決める。

`reason`、Contract gap、Challenger finding、コード、テストは新仕様を決めるauthorityではない。既存accepted Contract、Issue明示要求、記録された人の判断、accepted Decisionで決まらない場合は、一時的なDecision Requestとして問い、選択肢、影響、推奨、判断者を整理する。解決後の正本はDecisionとContractに置く。

## 5. 未知を先に潰す

SDK、DB、検索、通信、認証の未知は、機能全体を作る前に最小 probe で検証する。probe は製品コードの代用品ではなく、能力の事実を得るために使う。

## 6. 実装前にContractとauthorityを反証する

Contract resolve前に、ChallengerはIssue、Assessment、authority、Decision、Featureと上位Contractを独立に読み、判断漏れ、範囲超過、循環authority、未申告の仕様拡張を探す。findingはauthorityとして使わず、blockingならAssessmentまたはDecision Requestへ戻す。

## 7. Builder と実装後Challengerを分ける

Builderはresolved contract lockに従って実装する。ChallengerはBuilderの説明ではなくlock、raw diff、Mutation Graph、証拠から次を独立に試す。

- 境界値、空、重複、順序逆転
- 部分失敗、timeout、再試行、取消
- 同時実行と二重送信
- 型変換と serialize/deserialize
- 権限、テナント、情報漏えい
- 参照設計からの逸脱

Challenger が見つけた問題は、実装修正だけでなく契約や共通テストへ反映する。

## 8. 完了を証拠で判定する

契約項目ごとにテスト、probe、観測結果、Decisionを対応させる。証拠のない項目は完了にしない。残存リスクは隠さず、受容者と期限を記録する。
