import type { Metadata, Viewport } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import { headers } from "next/headers";
import "./globals.css";

const geistSans = Geist({
  variable: "--font-geist-sans",
  subsets: ["latin"],
});

const geistMono = Geist_Mono({
  variable: "--font-geist-mono",
  subsets: ["latin"],
});

export async function generateMetadata(): Promise<Metadata> {
  const requestHeaders = await headers();
  const host = requestHeaders.get("x-forwarded-host") ?? requestHeaders.get("host");
  const protocol = requestHeaders.get("x-forwarded-proto") ?? "https";
  const origin = host ? `${protocol}://${host}` : "https://github.com";
  const socialImage = `${origin}/og.png`;

  return {
    title: "Agentic Development Framework",
    description:
      "A repository control plane that keeps product decisions human and makes evidence durable.",
    openGraph: {
      title: "AI agents should implement decisions. Not make them.",
      description:
        "Connect specifications, human decisions, implementation, and evidence in every agent-assisted change.",
      type: "website",
      images: [{ url: socialImage, width: 1200, height: 630 }],
    },
    twitter: {
      card: "summary_large_image",
      title: "Agentic Development Framework",
      description: "Keep decisions human. Make evidence durable.",
      images: [socialImage],
    },
  };
}

export const viewport: Viewport = {
  themeColor: "#0d1516",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en">
      <body className={`${geistSans.variable} ${geistMono.variable}`}>
        {children}
      </body>
    </html>
  );
}
