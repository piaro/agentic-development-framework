using System.Net.Http;
class DotnetHttpService {
  async Task Notify(HttpClient httpClient, HttpRequestMessage request) {
    await httpClient.SendAsync(request);
  }
}
