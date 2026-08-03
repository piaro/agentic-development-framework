using Azure.Storage.Blobs;
class AzureBlobService {
  async Task Archive(BlobClient blobClient, Stream payload) {
    await blobClient.UploadAsync(payload);
  }
}
