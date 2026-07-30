def place_order(order: object) -> str:
    """Persist an accepted order in the source-of-truth table."""

    orders.insert(order)
    return "stored"
