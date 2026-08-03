import org.springframework.web.reactive.function.client.WebClient
fun notify(webClient: WebClient) {
  webClient.post().uri("/orders").retrieve()
}
