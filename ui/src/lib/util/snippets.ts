import type { Snippet } from "../api/types";

/** `{{name}}` placeholders (letters, digits, `_`, `-`), in order of first use. */
export function placeholders(command: string): string[] {
  const out: string[] = [];
  for (const m of command.matchAll(/\{\{\s*([\w-]+)\s*\}\}/g)) if (!out.includes(m[1])) out.push(m[1]);
  return out;
}

/** Replace every `{{name}}` with its value (missing values become empty). */
export function fillPlaceholders(command: string, values: Record<string, string>): string {
  return command.replace(/\{\{\s*([\w-]+)\s*\}\}/g, (_, name: string) => values[name] ?? "");
}

/** Text to type into a terminal: one line per command line, plus Enter when running. */
export function snippetInput(command: string, run: boolean): string {
  const text = command.replace(/\r\n?/g, "\n").replace(/\n+$/, "").replace(/\n/g, "\r");
  return run ? text + "\r" : text;
}

export type SuggestedSnippet = Omit<Snippet, "id">;

/** Read-only diagnostics for Linux servers, offered in the Snippets panel until added. */
export const SUGGESTED_SNIPPETS: SuggestedSnippet[] = [
  { name: "Disk usage", command: "df -h", description: "Free space on every mounted filesystem", tags: ["disk"] },
  {
    name: "Largest directories",
    command: "sudo du -xh {{path}} --max-depth=1 2>/dev/null | sort -rh | head -20",
    description: "What is using the space under a directory",
    tags: ["disk"],
  },
  { name: "Memory", command: "free -h", description: "RAM and swap in use", tags: ["memory"] },
  {
    name: "Top processes by memory",
    command: "ps aux --sort=-%mem | head -15",
    description: "The 15 processes using the most memory",
    tags: ["memory", "processes"],
  },
  {
    name: "Top processes by CPU",
    command: "ps aux --sort=-%cpu | head -15",
    description: "The 15 processes using the most CPU",
    tags: ["cpu", "processes"],
  },
  { name: "Load and uptime", command: "uptime", description: "Load averages and time since boot", tags: ["cpu"] },
  { name: "Listening ports", command: "sudo ss -tulpn", description: "Open TCP/UDP ports and the process behind each", tags: ["network"] },
  { name: "Network interfaces", command: "ip -brief address", description: "Interfaces, state and addresses", tags: ["network"] },
  {
    name: "Service status",
    command: "systemctl status {{service}} --no-pager",
    description: "State and recent log lines of a systemd service",
    tags: ["systemd"],
  },
  {
    name: "Failed services",
    command: "systemctl --failed --no-pager",
    description: "systemd units that failed to start",
    tags: ["systemd"],
  },
  {
    name: "Service logs",
    command: "journalctl -u {{service}} -n 100 --no-pager",
    description: "Last 100 log lines of a systemd service",
    tags: ["systemd", "logs"],
  },
  {
    name: "Recent errors",
    command: "journalctl -p err -b --no-pager | tail -50",
    description: "Errors logged since the last boot",
    tags: ["logs"],
  },
  { name: "Kernel messages", command: "sudo dmesg -T | tail -50", description: "Latest kernel log (hardware, OOM kills)", tags: ["logs"] },
  {
    name: "OS and kernel",
    command: "cat /etc/os-release | head -4; uname -r",
    description: "Distribution, version and kernel",
    tags: ["system"],
  },
  { name: "Who is logged in", command: "w", description: "Logged-in users and what they run", tags: ["system"] },
  {
    name: "Docker containers",
    command: 'docker ps --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}"',
    description: "Running containers with status and ports",
    tags: ["docker"],
  },
  {
    name: "Container logs",
    command: "docker logs --tail 100 {{container}}",
    description: "Last 100 log lines of a container",
    tags: ["docker", "logs"],
  },
  { name: "Nginx config test", command: "sudo nginx -t", description: "Check the nginx configuration for errors", tags: ["nginx"] },
  {
    name: "Certificate expiry",
    command: "echo | openssl s_client -connect {{host}}:443 -servername {{host}} 2>/dev/null | openssl x509 -noout -dates",
    description: "Validity dates of a site's TLS certificate",
    tags: ["tls", "network"],
  },
];
