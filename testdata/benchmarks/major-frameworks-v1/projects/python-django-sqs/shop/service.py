def place_order(order, sqs):
    order.save()
    sqs.send_message(QueueUrl="orders", MessageBody="created")
