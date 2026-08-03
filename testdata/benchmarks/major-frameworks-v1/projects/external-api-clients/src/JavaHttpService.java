import java.net.http.HttpClient;
class JavaHttpService {
  void notify(HttpClient httpClient, HttpRequest request, BodyHandler handler) {
    httpClient.sendAsync(request, handler);
  }
}
