import { useState, useEffect } from 'react';
import { Bot, Zap, Layers, Wrench, ChevronDown, ChevronUp } from 'lucide-react';
import type { AgentEntry } from '@/types/api';
import { getAgents } from '@/lib/api';

function AgentCard({ agent }: { agent: AgentEntry }) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className="bg-gray-900 rounded-xl border border-gray-800 hover:border-gray-700 transition-colors">
      {/* Header */}
      <div className="p-5">
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-center gap-3 min-w-0">
            <div className="h-9 w-9 rounded-lg bg-blue-900/50 border border-blue-700/40 flex items-center justify-center flex-shrink-0">
              <Bot className="h-4 w-4 text-blue-400" />
            </div>
            <div className="min-w-0">
              <h4 className="text-sm font-semibold text-white truncate">{agent.id}</h4>
              <p className="text-xs text-gray-500 mt-0.5">
                {agent.provider} · {agent.model}
              </p>
            </div>
          </div>
          <div className="flex items-center gap-2 flex-shrink-0">
            {agent.agentic && (
              <span className="inline-flex items-center gap-1 px-2 py-1 rounded-full text-xs font-medium bg-green-900/40 text-green-400 border border-green-700/50">
                <Zap className="h-3 w-3" />
                Agentic
              </span>
            )}
            <button
              onClick={() => setExpanded((v) => !v)}
              className="p-1 text-gray-500 hover:text-gray-300 transition-colors"
              aria-label="Toggle details"
            >
              {expanded ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
            </button>
          </div>
        </div>

        {/* Stats row */}
        <div className="mt-4 flex flex-wrap gap-4">
          <div className="flex items-center gap-1.5 text-xs text-gray-400">
            <Layers className="h-3.5 w-3.5 text-gray-500" />
            <span>Depth <span className="text-white font-medium">{agent.max_depth}</span></span>
          </div>
          <div className="flex items-center gap-1.5 text-xs text-gray-400">
            <Zap className="h-3.5 w-3.5 text-gray-500" />
            <span>Iterations <span className="text-white font-medium">{agent.max_iterations}</span></span>
          </div>
          {agent.has_system_prompt && (
            <div className="flex items-center gap-1.5 text-xs text-gray-400">
              <span className="h-1.5 w-1.5 rounded-full bg-blue-500 inline-block" />
              <span>Custom prompt</span>
            </div>
          )}
          {agent.workspace_dir && (
            <div className="flex items-center gap-1.5 text-xs text-gray-400">
              <span className="h-1.5 w-1.5 rounded-full bg-purple-500 inline-block" />
              <span>Workspace</span>
            </div>
          )}
        </div>
      </div>

      {/* Expanded tools section */}
      {expanded && (
        <div className="border-t border-gray-800 px-5 py-4">
          <div className="flex items-center gap-1.5 mb-3">
            <Wrench className="h-3.5 w-3.5 text-gray-500" />
            <span className="text-xs font-medium text-gray-400">
              Tools ({agent.allowed_tools.length})
            </span>
          </div>
          {agent.allowed_tools.length === 0 ? (
            <p className="text-xs text-gray-600">No tools configured</p>
          ) : (
            <div className="flex flex-wrap gap-1.5">
              {agent.allowed_tools.map((tool) => (
                <span
                  key={tool}
                  className="px-2 py-0.5 rounded-md text-xs bg-gray-800 text-gray-300 border border-gray-700"
                >
                  {tool}
                </span>
              ))}
            </div>
          )}
          {agent.workspace_dir && (
            <div className="mt-3">
              <p className="text-xs text-gray-500">
                Workspace: <span className="text-gray-300 font-mono">{agent.workspace_dir}</span>
              </p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export default function Agents() {
  const [agents, setAgents] = useState<AgentEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getAgents()
      .then(setAgents)
      .catch((err: Error) => setError(err.message))
      .finally(() => setLoading(false));
  }, []);

  if (error) {
    return (
      <div className="p-6">
        <div className="rounded-lg bg-red-900/30 border border-red-700 p-4 text-red-300">
          Failed to load agents: {error}
        </div>
      </div>
    );
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="animate-spin rounded-full h-8 w-8 border-2 border-blue-500 border-t-transparent" />
      </div>
    );
  }

  const sorted = [...agents].sort((a, b) => a.id.localeCompare(b.id));

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center gap-2">
        <Bot className="h-5 w-5 text-blue-400" />
        <h2 className="text-base font-semibold text-white">
          Agents ({agents.length})
        </h2>
      </div>

      {/* Cards grid */}
      {sorted.length === 0 ? (
        <div className="bg-gray-900 rounded-xl border border-gray-800 p-8 text-center">
          <Bot className="h-10 w-10 text-gray-600 mx-auto mb-3" />
          <p className="text-gray-400">No agents configured.</p>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
          {sorted.map((agent) => (
            <AgentCard key={agent.id} agent={agent} />
          ))}
        </div>
      )}
    </div>
  );
}
