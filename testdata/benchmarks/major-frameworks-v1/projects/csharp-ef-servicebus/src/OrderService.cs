class OrderService {
  async Task PlaceOrder(Order order, DbContext db, ServiceBusSender sender) {
    await db.SaveChangesAsync();
    await sender.SendMessageAsync(new ServiceBusMessage("created"));
  }
}
