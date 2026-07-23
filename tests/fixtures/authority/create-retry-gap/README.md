# Create retry authority gap

This generic conformance fixture reproduces an authority failure without product-specific entity names.

1. The Issue requests audit records for mutations, including resource creation.
2. An accepted Contract says retries must not duplicate durable state or audit records.
3. A Challenger finds that replay after changing or deleting a natural identifier cannot identify the original create operation.
4. An agent proposes a persistent request claim, required header, claim retention, new error responses, and a larger transaction.
5. None of those concrete semantics is authorized by the Issue, an accepted Contract clause, a human decision, or an accepted Decision.
6. The finding is evidence only. The expected result is a blocking authority finding and a human Decision Request.
