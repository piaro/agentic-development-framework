# Standard Signal Domains

Signal Domain Catalogは、Detectorが出力できるSignal、必要なbinding、入力となるtyped repository factを固定する組込み契約です。domainはSignalの分類であり、source codeの意味を推測したり、Ruleを暗黙に選択したりしません。

## Catalog v1

| Domain | Signal | 必須binding | Repository fact |
|---|---|---|---|
| `data-persistence` | `persistent-data-write` | `data`, `operation` | `db_write` |
| `distributed-integration` | `distributed-effect` | `integration`, `operation` | `message_publish` |
| `distributed-integration` | `message-or-event-publish` | `integration`, `operation` | `message_publish` |

`message_publish`は、一般的な分散副作用と、より具体的なmessage/event publishの両方を出力します。Ruleは従来どおり個別のSignal IDを指定します。domain全体を指定するwildcardはありません。

## 確認方法

```sh
agentic catalog signal-domains --format text
agentic catalog signal-domains --format json
```

JSON形式は`schemas/catalog/v1/signal-domain-catalog.schema.json`に従い、catalog本体のcanonical digestを含みます。固定期待値は`golden/v1/signal-domain-catalog.json`です。

## Registry境界

組込み定義は起動時に`SignalCatalogRegistry`へ読み込み、ID重複、domain参照、Detector identity、binding、factからSignalへの参照を一度検証します。Detector、Rule Compiler、Application、`catalog signal-domains`は同じRegistry APIを使用します。ApplicationはRule compileに使用したRegistryを保持し、typed fact検出にも同じinstanceを渡すため、異なるCatalogを誤って参照しません。

現在Registryへ投入できるのは組込み定義だけです。所有型と注入経路は用意していますが、外部YAML、Framework Release Catalog、Project Catalogの読込みはまだ許可していません。次の段階でSchema検証、namespace、決定的merge、署名・lock固定を追加してから有効化します。

## 追加方針

新しいdomainやSignalを追加する場合は、次を同時にreviewします。

- domainとSignalの責務が既存項目と重複しないこと
- Signalが要求する論理binding
- typed repository factからSignalへの明示変換
- source observationとBinding Recordが、そのfactを根拠付きで生成できること
- Rule、Schema、golden、Detector benchmark、利用案内の更新

framework固有APIのmethod名だけで意味を決めません。安全に分類できない呼出しはBinding Recordによるreviewを要求し、解析不能・未対応の入力はcoverage gapとして停止します。将来の候補である認可変更、機密data access、外部API呼出し等も、根拠となるtyped factとbinding契約が定義されるまでは標準domainへ追加しません。
