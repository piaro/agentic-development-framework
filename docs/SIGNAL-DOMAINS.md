# Standard Signal Domains

Signal Domain Catalogは、Detectorが出力できるSignal、必要なbinding、入力となるtyped repository factを固定する組込み契約です。domainはSignalの分類であり、source codeの意味を推測したり、Ruleを暗黙に選択したりしません。

## Catalog v3

| Domain | Signal | 必須binding | Repository fact |
|---|---|---|---|
| `data-persistence` | `persistent-data-write` | `data`, `operation` | `db_write` |
| `data-persistence` | `persistent-data-write` | `data`, `operation` | `object_write` |
| `data-persistence` | `object-storage-write` | `data`, `operation` | `object_write` |
| `distributed-integration` | `distributed-effect` | `integration`, `operation` | `message_publish` |
| `distributed-integration` | `distributed-effect` | `integration`, `operation` | `external_call` |
| `distributed-integration` | `external-system-call` | `integration`, `operation` | `external_call` |
| `distributed-integration` | `message-or-event-publish` | `integration`, `operation` | `message_publish` |
| `security-boundary` | `authorization-control-change` | `authorization`, `operation` | `authorization_change` |
| `security-boundary` | `sensitive-data-access` | `data`, `operation` | `sensitive_data_access` |

`message_publish`と`external_call`は一般的な`distributed-effect`も出力し、`object_write`は一般的な`persistent-data-write`も出力します。このため既存の汎用Ruleを維持しながら、用途を限定したRuleを追加できます。Ruleは従来どおり個別のSignal IDを指定し、domain全体を指定するwildcardはありません。

`external_call`と`object_write`はmethod名から自動分類しません。導入先Projectで、実際の呼出しを確認したうえでMethod Bindingへ明示します。

```yaml
resources:
  payment_client:
    logical_refs:
      integration: integration.payment-provider
    owner: team.ordering
    authority_ref: decision.repository-bindings
  archive_bucket:
    logical_refs:
      integration: integration.amazon-s3
      data: data.order-archive
    owner: team.ordering
    authority_ref: decision.repository-bindings
methods:
  payment_client.request:
    fact_kinds: [external_call]
    owner: team.ordering
    authority_ref: decision.repository-bindings
  archive_bucket.put_object:
    fact_kinds: [external_call, object_write]
    owner: team.ordering
    authority_ref: decision.repository-bindings
```

標準Ruleは、どちらのSecurity Signalにも次のゲートを適用します。

| Phase | Role | Requirement | Assurance |
|---|---|---|---|
| before-build | Analyst | operation境界とSecurity Contractを確認 | attestation |
| before-build | Challenger | Security設計を反証 | attestation |
| before-merge | Builder | 現在revisionのテスト・probe証拠を提出 | evidence-backed |
| before-merge | Challenger | Security実装を反証 | attestation |

認可境界やデータ分類の意味は機械判定せず、accepted DecisionとContractを根拠に人またはAgentが判断します。一方、提出されたEvidenceについては、現在revision、成功終了、Contract条項の網羅、artifact digestをKernelが検査します。

対応するresource Bindingは、`external_call`では`integration.*`、`object_write`では`data.*`の論理refを要求します。1つの呼出しに複数kindがある場合、resourceの`logical_refs`へ両方を記録し、すべてを検証してからfactを一括生成します。未知のkind、必要なMethod Bindingがない呼出し、不正または不足した論理refはfail-closedで停止します。

`project observe`は、主要HTTP clientとAmazon S3・Google Cloud Storage・Azure Blob Storageについて、manifest・import・型・receiverの根拠がある呼出しを非authoritativeな候補として提示します。`suggested_fact_kinds`があってもBindingへ自動転記せず、reviewerが意味を確認します。明確なObject Storage uploadは`external_call`と`object_write`の両方を提示します。JavaScript版S3の`client.send`はCommandによって読書きが変わるため空listにし、receiverのないbare `fetch()`も安定したresource Bindingを作れないため対象外です。

Security factは名前だけでは判定しません。たとえば`grant`が認可変更か、`find`の対象が機密dataかはProject固有だからです。導入先がaccepted Decisionに基づき、次のように明示した場合だけSignalを生成します。

```yaml
resources:
  permissions:
    logical_refs:
      authorization: authorization.order-administration
    owner: team.security
    authority_ref: decision.repository-bindings
  customers:
    logical_refs:
      data: data.customer-pii
    owner: team.security
    authority_ref: decision.repository-bindings
methods:
  permissions.grant:
    fact_kinds: [authorization_change]
    owner: team.security
    authority_ref: decision.repository-bindings
  customers.find:
    fact_kinds: [sensitive_data_access]
    owner: team.security
    authority_ref: decision.repository-bindings
```

## 確認方法

```sh
adf catalog signal-domains --format text
adf catalog signal-domains --format json
```

JSON形式は`schemas/catalog/v1/signal-domain-catalog.schema.json`に従い、catalog本体のcanonical digestを含みます。固定期待値は`golden/v1/signal-domain-catalog.json`です。

## Registry境界

組込み定義は起動時に`SignalCatalogRegistry`へ読み込み、ID重複、domain参照、Detector identity、binding、factからSignalへの参照を一度検証します。Git Repository Adapter、Detector、Rule Compiler、Application、`catalog signal-domains`は同じRegistry APIを使用します。実Projectでは、Method Bindingの検証、typed fact生成、Rule compile、Signal検出へ同じRegistryを渡すため、異なるCatalogを誤って参照しません。

Signal Domain Catalog Registryへ投入できるのは、引き続き組込み定義だけです。Project固有のSignalやfact変換を外部YAMLから追加することはできません。

Framework固有のmethod候補は別のFramework Detection Catalogで扱います。このCatalogは署名済みFramework Releaseのassetとしてだけ追加でき、namespace、重複rule、対応言語、fact kindを検証します。導入Project内の任意fileは読み込みません。Catalogが提示する候補も非authoritativeであり、review済みBindingなしではSignalを生成しません。

## 追加方針

新しいdomainやSignalを追加する場合は、次を同時にreviewします。

- domainとSignalの責務が既存項目と重複しないこと
- Signalが要求する論理binding
- typed repository factからSignalへの明示変換
- source observationとBinding Recordが、そのfactを根拠付きで生成できること
- Rule、Schema、golden、Detector benchmark、利用案内の更新

framework固有APIのmethod名だけで意味を決めません。安全に分類できない呼出しはBinding Recordによるreviewを要求し、解析不能・未対応の入力はcoverage gapとして停止します。
