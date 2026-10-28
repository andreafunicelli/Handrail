import { describe, expect, it } from "vitest";
import { parseEditorCommand, resolveEditorPath } from "./editorCommand";

describe("parseEditorCommand", () => {
  it("recognizes one file opened in a terminal editor", () => {
    expect(parseEditorCommand("nano /etc/hosts")).toEqual({ editor: "nano", file: "/etc/hosts", sudo: false });
    expect(parseEditorCommand("  vim +12 app.conf ")).toEqual({ editor: "vim", file: "app.conf", sudo: false });
    expect(parseEditorCommand("sudo nano -w /etc/nginx/nginx.conf")).toEqual({ editor: "nano", file: "/etc/nginx/nginx.conf", sudo: true });
    expect(parseEditorCommand("sudoedit /etc/fstab")).toEqual({ editor: "sudoedit", file: "/etc/fstab", sudo: true });
    expect(parseEditorCommand(`vi 'my file.txt'`)?.file).toBe("my file.txt");
    expect(parseEditorCommand(`vi "my file.txt"`)?.file).toBe("my file.txt");
    expect(parseEditorCommand("vi my\\ file.txt")?.file).toBe("my file.txt");
    expect(parseEditorCommand("vim -- -odd-name")?.file).toBe("-odd-name");
  });

  it("leaves anything else to the terminal", () => {
    for (const line of [
      "",
      "nano",
      "vim -R",
      "nano a b",
      "nano -T 4 file",
      "nano ~/.bashrc",
      "vim $(which x)",
      'nano "$HOME/x"',
      "nano *.conf",
      "nano x; rm y",
      "cat x | vim -",
      "nano x > y",
      "EDITOR=vim nano x",
      "sudo -u www nano x",
      "sudo sudoedit x",
      "less /etc/hosts",
      "nanox /etc/hosts",
      "nano 'unterminated",
    ]) {
      expect(parseEditorCommand(line), line).toBeNull();
    }
  });
});

describe("resolveEditorPath", () => {
  it("makes the path absolute against the shell's folder", () => {
    expect(resolveEditorPath("/etc/./nginx//nginx.conf", null)).toBe("/etc/nginx/nginx.conf");
    expect(resolveEditorPath("app.conf", "/srv/app")).toBe("/srv/app/app.conf");
    expect(resolveEditorPath("./x", "/srv/app")).toBe("/srv/app/x");
    expect(resolveEditorPath("x", null)).toBeNull();
    // `..` after a symlink is not the parent folder of the path: left to the shell.
    expect(resolveEditorPath("../x", "/srv/app")).toBeNull();
    expect(resolveEditorPath("/srv/link/../config", null)).toBeNull();
  });
});
