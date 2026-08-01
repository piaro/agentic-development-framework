import boto3
s3 = boto3.client("s3")
def archive(payload):
    s3.put_object(Bucket="orders", Key="latest", Body=payload)
