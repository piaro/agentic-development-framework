def publish_order(event: object) -> str:
    """Publish the order-created event to the configured queue."""

    order_events.publish(event)
    return "queued"
