import httpx

def lookup():
    httpx.get("https://example.test/orders")
