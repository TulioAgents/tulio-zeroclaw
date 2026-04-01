import { useState, useEffect, useRef } from 'react';
import { Kanban, Circle, Loader2, CheckCircle2, Clock, FolderOpen, Bot, RefreshCw } from 'lucide-react';
import type { KanbanTask, KanbanProject } from '@/types/api';
import { getKanban } from '@/lib/api';
import { getToken } from '@/lib/auth';

// ---------------------------------------------------------------------------
// Agent live-status tracker via SSE
// ---------------------------------------------------------------------------

function useAgentLiveStatus() {
  const [liveAgents, setLiveAgents] = useState<Set<string>>(new Set());
  const esRef = useRef<EventSource | null>(null);

  useEffect(() => {
    const token = getToken();
    const url = token ? `/api/events?token=${encodeURIComponent(token)}` : '/api/events';

    // Some gateways accept token as header; SSE doesn't support headers in browser
    // so we rely on no-auth or query-param fallback. If 401 we just skip live status.
    const es = new EventSource(url);
    esRef.current = es;

    es.onmessage = (e) => {
      try {
        const event = JSON.parse(e.data) as { type: string; model?: string };
        if (event.type === 'agent_start' && event.model) {
          // model is like "hint:normal" — we can't map to agent_id directly from SSE
          // because the server doesn't emit the agent id. We mark a generic "active" state.
          setLiveAgents((prev) => {
            const next = new Set(prev);
            next.add('__any__');
            return next;
          });
        } else if (event.type === 'agent_end') {
          setLiveAgents((prev) => {
            const next = new Set(prev);
            next.delete('__any__');
            return next;
          });
        }
        // tool_call_start carries the tool name, which agents emit when running
        if (event.type === 'tool_call_start' || event.type === 'llm_request') {
          setLiveAgents((prev) => {
            const next = new Set(prev);
            next.add('__active__');
            return next;
          });
          // auto-clear after 10s of no activity
        }
      } catch {
        // ignore parse errors
      }
    };

    es.onerror = () => {
      es.close();
    };

    return () => {
      es.close();
      esRef.current = null;
    };
  }, []);

  const isSystemActive = liveAgents.has('__any__') || liveAgents.has('__active__');
  return { isSystemActive };
}

// ---------------------------------------------------------------------------
// Column config
// ---------------------------------------------------------------------------

type ColumnId = 'todo' | 'in_progress' | 'done';

const COLUMNS: { id: ColumnId; label: string; icon: typeof Circle; color: string; border: string; badge: string }[] = [
  {
    id: 'todo',
    label: 'To Do',
    icon: Circle,
    color: 'text-gray-400',
    border: 'border-gray-700',
    badge: 'bg-gray-800 text-gray-300',
  },
  {
    id: 'in_progress',
    label: 'In Progress',
    icon: Loader2,
    color: 'text-blue-400',
    border: 'border-blue-700/50',
    badge: 'bg-blue-900/40 text-blue-300',
  },
  {
    id: 'done',
    label: 'Done',
    icon: CheckCircle2,
    color: 'text-green-400',
    border: 'border-green-700/30',
    badge: 'bg-green-900/30 text-green-400',
  },
];

// ---------------------------------------------------------------------------
// Agent color map (consistent per agent id)
// ---------------------------------------------------------------------------

const AGENT_COLORS: Record<string, string> = {
  cto: 'bg-purple-900/50 text-purple-300 border-purple-700/40',
  manager: 'bg-blue-900/50 text-blue-300 border-blue-700/40',
  po: 'bg-pink-900/50 text-pink-300 border-pink-700/40',
  'tech-lead': 'bg-yellow-900/50 text-yellow-300 border-yellow-700/40',
  'staff-fullstack': 'bg-cyan-900/50 text-cyan-300 border-cyan-700/40',
  'sr-fullstack': 'bg-teal-900/50 text-teal-300 border-teal-700/40',
  mobile: 'bg-orange-900/50 text-orange-300 border-orange-700/40',
  qa: 'bg-red-900/50 text-red-300 border-red-700/40',
  devops: 'bg-lime-900/50 text-lime-300 border-lime-700/40',
  unknown: 'bg-gray-800 text-gray-400 border-gray-700',
};

function agentColor(id: string) {
  return AGENT_COLORS[id] ?? AGENT_COLORS.unknown;
}

// ---------------------------------------------------------------------------
// Task card
// ---------------------------------------------------------------------------

function TaskCard({ task, isSystemActive }: { task: KanbanTask; isSystemActive: boolean }) {
  const isInProgress = task.status === 'in_progress';
  const agentBusy = isSystemActive && isInProgress;

  return (
    <div
      className={`bg-gray-900 rounded-xl border p-4 transition-all ${
        isInProgress ? 'border-blue-700/40' : 'border-gray-800 hover:border-gray-700'
      }`}
    >
      {/* Task title */}
      <p className="text-sm font-medium text-white leading-snug">{task.title}</p>

      {/* Meta row */}
      <div className="mt-3 flex flex-wrap items-center gap-2">
        {/* Agent badge */}
        <span
          className={`inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium border ${agentColor(task.agent_id)}`}
        >
          {/* Live dot */}
          {agentBusy ? (
            <span className="h-1.5 w-1.5 rounded-full bg-green-400 animate-pulse inline-block" />
          ) : (
            <Bot className="h-3 w-3 opacity-60" />
          )}
          {task.agent_id !== 'unknown' ? task.agent_id : task.owner || 'unassigned'}
        </span>

        {/* Project */}
        <span className="inline-flex items-center gap-1 text-xs text-gray-500">
          <FolderOpen className="h-3 w-3" />
          {task.project}
        </span>

        {/* Task ID if present */}
        {task.id && !task.id.includes('-') && (
          <span className="text-xs text-gray-600 font-mono">{task.id}</span>
        )}

        {/* Archived tag */}
        {task.archived && (
          <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-xs bg-gray-800 text-gray-500 border border-gray-700">
            <Clock className="h-3 w-3" />
            archived
          </span>
        )}
      </div>

      {/* Change ID */}
      <p className="mt-2 text-xs text-gray-600 truncate">{task.change_id}</p>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Column
// ---------------------------------------------------------------------------

function Column({
  col,
  tasks,
  isSystemActive,
}: {
  col: (typeof COLUMNS)[number];
  tasks: KanbanTask[];
  isSystemActive: boolean;
}) {
  const Icon = col.icon;

  return (
    <div className="flex flex-col min-w-0">
      {/* Column header */}
      <div className={`flex items-center gap-2 mb-3 pb-3 border-b ${col.border}`}>
        <Icon className={`h-4 w-4 flex-shrink-0 ${col.color} ${col.id === 'in_progress' ? 'animate-spin' : ''}`} style={col.id === 'in_progress' ? { animationDuration: '3s' } : {}} />
        <span className="text-sm font-semibold text-white">{col.label}</span>
        <span className={`ml-auto px-2 py-0.5 rounded-full text-xs font-medium ${col.badge}`}>
          {tasks.length}
        </span>
      </div>

      {/* Cards */}
      <div className="flex flex-col gap-3">
        {tasks.length === 0 ? (
          <div className="rounded-xl border border-dashed border-gray-800 p-4 text-center">
            <p className="text-xs text-gray-600">No tasks</p>
          </div>
        ) : (
          tasks.map((task) => (
            <TaskCard key={`${task.project}-${task.change_id}-${task.id}`} task={task} isSystemActive={isSystemActive} />
          ))
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main page
// ---------------------------------------------------------------------------

export default function KanbanBoard() {
  const [tasks, setTasks] = useState<KanbanTask[]>([]);
  const [projects, setProjects] = useState<KanbanProject[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filterProject, setFilterProject] = useState<string>('all');
  const [filterAgent, setFilterAgent] = useState<string>('all');
  const [hideArchived, setHideArchived] = useState(true);

  const { isSystemActive } = useAgentLiveStatus();

  const load = () => {
    setLoading(true);
    setError(null);
    getKanban()
      .then((board) => {
        setProjects(board.projects);
        setTasks(board.tasks);
      })
      .catch((err: Error) => setError(err.message))
      .finally(() => setLoading(false));
  };

  useEffect(() => { load(); }, []);

  // Derive unique agent ids for filter
  const agentIds = Array.from(new Set(tasks.map((t) => t.agent_id))).sort();

  // Apply filters
  const filtered = tasks.filter((t) => {
    if (filterProject !== 'all' && t.project !== filterProject) return false;
    if (filterAgent !== 'all' && t.agent_id !== filterAgent) return false;
    if (hideArchived && t.archived) return false;
    return true;
  });

  const byColumn = (col: ColumnId) => filtered.filter((t) => t.status === col);

  if (error) {
    return (
      <div className="p-6">
        <div className="rounded-lg bg-red-900/30 border border-red-700 p-4 text-red-300">
          Failed to load kanban: {error}
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

  return (
    <div className="p-6 space-y-5">
      {/* Header */}
      <div className="flex items-center gap-3 flex-wrap">
        <div className="flex items-center gap-2">
          <Kanban className="h-5 w-5 text-blue-400" />
          <h2 className="text-base font-semibold text-white">Kanban Board</h2>
        </div>

        {/* System live indicator */}
        <div className="flex items-center gap-1.5 ml-1">
          <span
            className={`h-2 w-2 rounded-full flex-shrink-0 ${
              isSystemActive ? 'bg-green-400 animate-pulse' : 'bg-gray-600'
            }`}
          />
          <span className={`text-xs font-medium ${isSystemActive ? 'text-green-400' : 'text-gray-500'}`}>
            {isSystemActive ? 'Agent active' : 'No agent running'}
          </span>
        </div>

        <div className="ml-auto flex items-center gap-2 flex-wrap">
          {/* Refresh */}
          <button
            onClick={load}
            className="p-1.5 text-gray-500 hover:text-gray-300 transition-colors rounded-lg hover:bg-gray-800"
            title="Refresh"
          >
            <RefreshCw className="h-4 w-4" />
          </button>

          {/* Hide archived toggle */}
          <button
            onClick={() => setHideArchived((v) => !v)}
            className={`px-3 py-1.5 rounded-lg text-xs font-medium transition-colors border ${
              hideArchived
                ? 'bg-gray-800 text-gray-400 border-gray-700'
                : 'bg-blue-600 text-white border-blue-500'
            }`}
          >
            {hideArchived ? 'Show archived' : 'Hide archived'}
          </button>
        </div>
      </div>

      {/* Filters */}
      <div className="flex flex-wrap gap-3">
        {/* Project filter */}
        <div className="flex flex-wrap gap-1.5 items-center">
          <FolderOpen className="h-3.5 w-3.5 text-gray-500" />
          {['all', ...projects.filter((p) => p.status !== 'closed').map((p) => p.name)].map((p) => (
            <button
              key={p}
              onClick={() => setFilterProject(p)}
              className={`px-3 py-1 rounded-lg text-xs font-medium transition-colors capitalize ${
                filterProject === p
                  ? 'bg-blue-600 text-white'
                  : 'bg-gray-900 text-gray-400 border border-gray-700 hover:bg-gray-800 hover:text-white'
              }`}
            >
              {p === 'all' ? 'All Projects' : p}
            </button>
          ))}
        </div>

        {/* Divider */}
        <div className="w-px bg-gray-800 self-stretch" />

        {/* Agent filter */}
        <div className="flex flex-wrap gap-1.5 items-center">
          <Bot className="h-3.5 w-3.5 text-gray-500" />
          {['all', ...agentIds].map((a) => (
            <button
              key={a}
              onClick={() => setFilterAgent(a)}
              className={`px-3 py-1 rounded-lg text-xs font-medium transition-colors ${
                filterAgent === a
                  ? 'bg-blue-600 text-white'
                  : `bg-gray-900 text-gray-400 border border-gray-700 hover:bg-gray-800 hover:text-white`
              }`}
            >
              {a === 'all' ? 'All Agents' : a}
            </button>
          ))}
        </div>
      </div>

      {/* Board */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
        {COLUMNS.map((col) => (
          <Column
            key={col.id}
            col={col}
            tasks={byColumn(col.id)}
            isSystemActive={isSystemActive}
          />
        ))}
      </div>
    </div>
  );
}
