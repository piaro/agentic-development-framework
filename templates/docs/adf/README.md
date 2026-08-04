# Agentic Development

このRepositoryでは、いま正しいとしていることを`contracts/`、そう決めた理由を`decisions/`、それが満たされている証拠を`evidence/`に置きます。

## 変更を始める

```sh
adf change init <change-id> --title "変更タイトル"
```

## 進め方

次にやることは`adf next <change-id>`が返します。エージェントはその1件を実行し、結果を提出して、また次を受け取ります。手順の順番を覚えておく必要はありません。

```sh
adf next <change-id>
```

エージェントの通常経路はMCPです。`adf mcp`を起動すると、同じやり取りを`adf_next`と`adf_submit`で行えます。

受け取った作業の役割に応じてSkillを使い分けます。

| 役割 | Skill | 担当する作業 |
|---|---|---|
| Analyst | `$adf-analyst` | 検出候補の確認、影響範囲と操作境界の確定、Contractの記入、人への判断依頼、回答の記録 |
| Builder | `$adf-builder` | 実装と、Contract条項に対応する証拠の記録 |
| Challenger | `$adf-challenger` | 実装前と実装後の反証 |

実装後の反証は、実装した文脈から独立した文脈で行います。

判定の理由を知りたいときは`adf explain <change-id>`を実行します。

## 決めてよいことの範囲

仕様を決めてよい根拠は、既存のaccepted Contract、依頼に明示された要求、記録された人の判断、accepted Decisionだけです。エージェントの推論、反証で見つけた指摘、Contractの不足、コード、テストは証拠であり、仕様を決める権限にはなりません。

権限のある根拠で決められない場合は、選択肢、影響、推奨、必要な決定者を添えて人へ戻します。人が答えたら、判断の理由を`decisions/`へ、そこから決まった現在の規範を`contracts/`へ記録します。質問そのものは一時的な情報なので、以降の成果物から参照しません。

Feature Contractから上位Contractを暗黙に変更しません。全体の判断が未確定なら、その変更を止めて先にContractを決めます。
