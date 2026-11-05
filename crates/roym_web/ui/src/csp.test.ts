import { describe, expect, it } from "vitest";
import { connectPolicy, installConnectPolicy } from "./csp";

describe("connectPolicy", () => {
  it("allows the Hub itself and its one auth origin", () => {
    expect(connectPolicy("http://auth.localhost:7960")).toBe(
      "connect-src 'self' http://auth.localhost:7960",
    );
    expect(connectPolicy("https://auth.example.com")).toBe(
      "connect-src 'self' https://auth.example.com",
    );
  });
});

describe("installConnectPolicy", () => {
  it("adds one policy tag to the head", () => {
    const before = document.head.querySelectorAll('meta[http-equiv="Content-Security-Policy"]').length;

    installConnectPolicy(document, "https://auth.example.com");

    const tags = document.head.querySelectorAll('meta[http-equiv="Content-Security-Policy"]');
    expect(tags.length).toBe(before + 1);
    expect(tags[tags.length - 1].getAttribute("content")).toBe(
      "connect-src 'self' https://auth.example.com",
    );
  });
});
