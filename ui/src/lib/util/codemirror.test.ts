import { describe, expect, it } from "vitest";
import { detectEol, detectIndent, detectLanguage } from "./codemirror";

describe("detectLanguage", () => {
  it("uses the extension, then the whole name", () => {
    expect(detectLanguage("/srv/app/main.py")).toBe("python");
    expect(detectLanguage("/srv/app/index.PHP")).toBe("php");
    expect(detectLanguage("/srv/app/Dockerfile")).toBe("dockerfile");
    expect(detectLanguage("/srv/app/Dockerfile.prod")).toBe("dockerfile");
    expect(detectLanguage("/home/me/.bashrc")).toBe("shell");
    expect(detectLanguage("/srv/app/.env.local")).toBe("ini");
    expect(detectLanguage("/etc/systemd/system/app.service")).toBe("ini");
  });

  it("treats .conf under nginx/ as nginx", () => {
    expect(detectLanguage("/etc/nginx/sites-available/site.conf")).toBe("nginx");
    expect(detectLanguage("/etc/nginx/sites-available/default", "server {\n}")).toBe("nginx");
    expect(detectLanguage("/etc/sysctl.conf")).toBe("ini");
  });

  it("reads #! lines of files without an extension", () => {
    expect(detectLanguage("/usr/local/bin/deploy", "#!/bin/bash\necho hi")).toBe("shell");
    expect(detectLanguage("/usr/local/bin/tool", "#!/usr/bin/env python3\nprint(1)")).toBe("python");
    expect(detectLanguage("/usr/local/bin/x", "#!/usr/bin/env -S node --harmony\n")).toBe("javascript");
    expect(detectLanguage("/srv/notes", "just text")).toBe("text");
  });
});

describe("detectIndent", () => {
  it("finds the step of space indentation", () => {
    expect(detectIndent("a:\n  b:\n    c: 1\n  d: 2\n")).toEqual({ tabs: false, size: 2 });
    expect(detectIndent("def f():\n    if x:\n        return 1\n    return 2\n")).toEqual({ tabs: false, size: 4 });
  });

  it("finds tabs, and falls back when nothing is indented", () => {
    expect(detectIndent("server {\n\tlisten 80;\n\troot /srv;\n}\n")).toEqual({ tabs: true, size: 4 });
    expect(detectIndent("a\nb\n", { tabs: false, size: 2 })).toEqual({ tabs: false, size: 2 });
  });
});

it("detectEol", () => {
  expect(detectEol("a\r\nb\r\n")).toBe("CRLF");
  expect(detectEol("a\nb\n")).toBe("LF");
});
