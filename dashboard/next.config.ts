import type { NextConfig } from "next";

// Static export: shadownetd serves dashboard/out directly, there is no Node
// server in production.
const config: NextConfig = {
  output: "export",
  images: { unoptimized: true },
  reactStrictMode: true,
  devIndicators: false,
  trailingSlash: false,
};

export default config;
