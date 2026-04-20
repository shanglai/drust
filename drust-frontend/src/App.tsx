import React, { useEffect, useMemo, useState } from "react";
import mermaid from "mermaid";
import { Sankey, Tooltip } from "recharts";

/**
 * Minimal frontend for Drust-core
 * 1) List rules and render selected rule as a Mermaid diagram
 * 2) Upload YAML, preview, and upsert to backend
 * 3) Poll and show execution graph as a Sankey (every 10s)
 *
 * Assumptions:
 * - Backend base URL is set via DRUST_API_BASE (env) or defaults to same origin
 * - /admin/rules -> [{ rule_id, versions:[{version, weight}] }]
 * - /admin/rules/:rule_id -> full RuleEntry with versions[n].nodes
 * - /admin/rules/upsert (POST { yaml })
 * - For Sankey, either: provide a public JSON URL (GCS) or a backend proxy endpoint
 */

//const API_BASE = (import.meta as any)?.env?.VITE_DRUST_API_BASE || "";
////const API_BASE = import.meta.env?.VITE_DRUST_API_BASE || "";
const API_BASE = "http://34.63.238.145:8080";

// ---- Types (align with backend) ----
type VersionNode = {
  id: string;
  type: string; // START | CASE | ALL | ANY | ACTION | CALL | RETURN
  next?: string;
  next_on_pass?: string;
  next_on_fail?: string;
  branches?: { when: string; next: string }[];
};

type RuleVersion = {
  version: string;
  weight: number;
  nodes?: VersionNode[]; // present when fetching a single rule
};

type RuleSummary = {
  rule_id: string;
  versions: { version: string; weight: number }[];
};

type RuleEntry = {
  id: string; // rule_id
  versions: RuleVersion[];
};

// ---- Mermaid helper ----
function toMermaid(ver: RuleVersion): string {
  if (!ver.nodes || ver.nodes.length === 0) return "graph TD\n  A[Empty]";
  const lines: string[] = ["flowchart TD"]; // Mermaid flowchart is clearer than graph TD

  // Give each node a label with type
  const label = (n: VersionNode) => `${n.id}("${n.id}: ${n.type}")`;

  // Map for quick lookup by id
  const idSet = new Set(ver.nodes.map(n => n.id));

  // Always render nodes so isolated ones still appear
  ver.nodes.forEach(n => lines.push(`${label(n)}`));

  // Edges by type
  for (const n of ver.nodes) {
    switch (n.type) {
      case "START":
        if (n.next) lines.push(`${n.id} --> ${n.next}`);
        break;
      case "ALL":
      case "ANY":
        if (n.next_on_pass) lines.push(`${n.id} -- pass --> ${n.next_on_pass}`);
        if (n.next_on_fail) lines.push(`${n.id} -- fail --> ${n.next_on_fail}`);
        break;
      case "CASE":
        (n.branches || []).forEach((b, i) => {
          lines.push(`${n.id} -- ${b.when.replaceAll("\"", "'")} --> ${b.next}`);
        });
        if (n.next) lines.push(`${n.id} -- default --> ${n.next}`);
        break;
      case "ACTION":
      case "CALL":
        if (n.next) lines.push(`${n.id} --> ${n.next}`);
        break;
      case "RETURN":
        // terminal
        break;
      default:
        // unknown types: do nothing
        break;
    }
  }

  return lines.join("\n");
}

// Render Mermaid string safely
function MermaidView({ def }: { def: string }) {
  const [svg, setSvg] = useState<string>("");
  useEffect(() => {
    let mounted = true;
    mermaid.initialize({ startOnLoad: false, theme: "default" });
    mermaid.render(`m_${Date.now()}`, def).then(({ svg }) => {
      if (mounted) setSvg(svg);
    }).catch(err => {
      setSvg(`<pre style=\"color:red\">${String(err)}</pre>`);
    });
    return () => { mounted = false };
  }, [def]);
  return <div className="w-full overflow-auto" dangerouslySetInnerHTML={{ __html: svg }} />;
}

// ---- Sankey types ----
type SankeyNode = { name: string };
// Recharts Sankey wants {source, target, value} as indices
// We'll accept a friendly JSON: { nodes:[{id}], links:[{sourceId,targetId,value}] }

type RawSankey = { nodes: { id: string }[]; links: { source: string; target: string; value: number }[] };

function prepareSankey(raw: RawSankey) {
  const indexById = new Map<string, number>();
  const nodes = raw.nodes.map((n, i) => { indexById.set(n.id, i); return { name: n.id } as SankeyNode });
  const links = raw.links.map(l => ({
    source: indexById.get(l.source) ?? 0,
    target: indexById.get(l.target) ?? 0,
    value: l.value,
  }));
  return { nodes, links };
}

// ---- Components ----
function RulesList({ onPick }: { onPick: (ruleId: string) => void }) {
  const [items, setItems] = useState<RuleSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    setLoading(true);
    fetch(`${API_BASE}/admin/rules`)
      .then(r => r.json())
      .then((data) => { if (live) setItems(data || []); })
      .catch(e => setErr(String(e)))
      .finally(() => setLoading(false));
    return () => { live = false };
  }, []);

  return (
    <div className="p-4 bg-white rounded-2xl shadow">
      <div className="text-xl font-semibold mb-2">Rules</div>
      {loading && <div className="text-sm opacity-70">Loading…</div>}
      {err && <div className="text-sm text-red-600">{err}</div>}
      <ul className="space-y-2">
        {items.map(item => (
          <li key={item.rule_id} className="flex items-center justify-between p-2 border rounded-xl">
            <div>
              <div className="font-medium">{item.rule_id}</div>
              <div className="text-xs opacity-70">versions: {item.versions.map(v => `${v.version}(${v.weight}%)`).join(", ")}</div>
            </div>
            <button className="px-3 py-1 rounded-xl bg-black text-white" onClick={() => onPick(item.rule_id)}>View</button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function RuleViewer({ ruleId }: { ruleId: string }) {
  const [entry, setEntry] = useState<RuleEntry | null>(null);
  const [verIdx, setVerIdx] = useState(0);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    setErr(null);
    setEntry(null);
    fetch(`${API_BASE}/admin/rules/${encodeURIComponent(ruleId)}`)
      .then(r => r.json())
      .then((data) => { if (live) setEntry(data); })
      .catch(e => setErr(String(e)));
    return () => { live = false };
  }, [ruleId]);

  const ver = entry?.versions?.[verIdx];
  const mermaidStr = useMemo(() => (ver ? toMermaid(ver) : "graph TD\n  A[Loading]") , [ver]);

  return (
    <div className="p-4 bg-white rounded-2xl shadow">
      <div className="flex items-center justify-between mb-2">
        <div className="text-xl font-semibold">{ruleId}</div>
        {entry && (
          <select className="border rounded-xl px-2 py-1" value={verIdx}
            onChange={e => setVerIdx(Number(e.target.value))}>
            {entry.versions.map((v, i) => (
              <option key={v.version} value={i}>{v.version} ({v.weight}%)</option>
            ))}
          </select>
        )}
      </div>
      {err && <div className="text-sm text-red-600">{err}</div>}
      <MermaidView def={mermaidStr} />
    </div>
  );
}

function RuleUpsert() {
  const [yaml, setYaml] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const onFile = async (f?: File) => {
    if (!f) return;
    const txt = await f.text();
    setYaml(txt);
  };

  const onSend = async () => {
    setBusy(true); setMsg(null);
    try {
      const r = await fetch(`${API_BASE}/admin/rules/upsert`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ yaml }),
      });
      if (r.ok) setMsg("Upsert OK"); else setMsg(`Upsert failed: ${r.status}`);
    } catch (e: any) { setMsg(String(e)); }
    finally { setBusy(false); }
  };

  return (
    <div className="p-4 bg-white rounded-2xl shadow space-y-3">
      <div className="text-xl font-semibold">Load / Upsert Rule YAML</div>
      <input type="file" accept=".yml,.yaml" onChange={e => onFile(e.target.files?.[0] || undefined)} />
      <textarea className="w-full h-48 border rounded-xl p-2 font-mono" value={yaml} onChange={e => setYaml(e.target.value)} />
      <div className="flex gap-2">
        <button disabled={!yaml || busy} className="px-3 py-1 rounded-xl bg-black text-white disabled:opacity-50" onClick={onSend}>
          {busy ? "Uploading…" : "Upsert"}
        </button>
        <button className="px-3 py-1 rounded-xl border" onClick={() => setYaml("")}>Clear</button>
      </div>
      {msg && <div className="text-sm">{msg}</div>}
    </div>
  );
}

function SankeyView({ jsonUrl }: { jsonUrl: string }) {
  const [raw, setRaw] = useState<RawSankey | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let stop = false;
    async function loadOnce() {
      try {
        const r = await fetch(jsonUrl, { cache: "no-cache" });
        if (!r.ok) throw new Error(`${r.status}`);
        const j = await r.json();
        if (!stop) setRaw(j);
      } catch (e: any) { setErr(String(e)); }
    }
    loadOnce();
    const h = setInterval(loadOnce, 10_000);
    return () => { stop = true; clearInterval(h); };
  }, [jsonUrl]);

  const data = useMemo(() => raw ? prepareSankey(raw) : { nodes: [], links: [] }, [raw]);

  return (
    <div className="p-4 bg-white rounded-2xl shadow">
      <div className="text-xl font-semibold mb-2">Execution Sankey</div>
      {err && <div className="text-sm text-red-600">{err}</div>}
      {data.nodes.length > 0 ? (
        <Sankey width={800} height={400} data={data} nodePadding={30} nodeWidth={15}>
          <Tooltip />
        </Sankey>
      ) : (
        <div className="text-sm opacity-70">No data</div>
      )}
    </div>
  );
}

export default function App() {
  const [picked, setPicked] = useState<string | null>(null);
  const [sankeyUrl, setSankeyUrl] = useState<string>("");

  return (
    <div className="min-h-screen bg-neutral-50 p-6 space-y-6">
      <h1 className="text-2xl font-bold">Drust Frontend MVP</h1>

      <div className="grid grid-cols-1 xl:grid-cols-2 gap-6">
        <RulesList onPick={setPicked} />
        {picked && <RuleViewer ruleId={picked} />}
      </div>

      <RuleUpsert />

      <div className="p-4 bg-white rounded-2xl shadow space-y-2">
        <div className="flex items-center gap-2">
          <div className="font-semibold">Sankey JSON URL</div>
          <input className="flex-1 border rounded-xl px-2 py-1" placeholder="https://storage.googleapis.com/<bucket>/path/to/sankey.json"
                 value={sankeyUrl} onChange={e => setSankeyUrl(e.target.value)} />
        </div>
        {sankeyUrl && <SankeyView jsonUrl={sankeyUrl} />}
        <div className="text-xs opacity-60">Tip: make the GCS object public for MVP or add a backend proxy that reads from GCS and returns JSON with proper CORS.</div>
      </div>
    </div>
  );
}
