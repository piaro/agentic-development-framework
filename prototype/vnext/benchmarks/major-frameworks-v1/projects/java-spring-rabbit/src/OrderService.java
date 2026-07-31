class OrderService {
  void placeOrder(Order order, OrderRepository repository, Channel channel) {
    repository.save(order);
    channel.basicPublish("", "orders", null, null);
  }
}
