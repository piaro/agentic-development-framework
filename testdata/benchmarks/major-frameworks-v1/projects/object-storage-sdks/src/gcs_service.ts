import { Storage } from "@google-cloud/storage";
export function archive(file: File, payload: string) {
  file.save(payload);
}
