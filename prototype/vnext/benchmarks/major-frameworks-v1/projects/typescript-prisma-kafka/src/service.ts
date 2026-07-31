export async function placeOrder(prisma: any, kafka: any, order: any) {
  await prisma.order.create({ data: order });
  await kafka.send({ topic: "orders", messages: [] });
}
