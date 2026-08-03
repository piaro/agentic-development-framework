import { PutObjectCommand, S3Client } from "@aws-sdk/client-s3";
export function archive(s3: S3Client, command: PutObjectCommand) {
  s3.send(command);
}
