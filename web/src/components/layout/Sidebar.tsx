import { useState, useEffect } from 'react';
import { NavLink } from 'react-router-dom';
import {
  LayoutDashboard,
  MessageSquare,
  Bot,
  Kanban,
  Wrench,
  Clock,
  Puzzle,
  Brain,
  Settings,
  DollarSign,
  Activity,
  Stethoscope,
  ChevronDown,
  ChevronRight,
} from 'lucide-react';
import { t } from '@/lib/i18n';
import { getAgents } from '@/lib/api';
import type { AgentEntry } from '@/types/api';

const bottomNavItems = [
  { to: '/agents', icon: Bot, labelKey: 'nav.agents' },
  { to: '/kanban', icon: Kanban, labelKey: 'nav.kanban' },
  { to: '/tools', icon: Wrench, labelKey: 'nav.tools' },
  { to: '/cron', icon: Clock, labelKey: 'nav.cron' },
  { to: '/integrations', icon: Puzzle, labelKey: 'nav.integrations' },
  { to: '/memory', icon: Brain, labelKey: 'nav.memory' },
  { to: '/config', icon: Settings, labelKey: 'nav.config' },
  { to: '/cost', icon: DollarSign, labelKey: 'nav.cost' },
  { to: '/logs', icon: Activity, labelKey: 'nav.logs' },
  { to: '/doctor', icon: Stethoscope, labelKey: 'nav.doctor' },
];

function navLinkClass(isActive: boolean) {
  return [
    'flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm font-medium transition-colors',
    isActive
      ? 'bg-blue-600 text-white'
      : 'text-gray-300 hover:bg-gray-800 hover:text-white',
  ].join(' ');
}

export default function Sidebar() {
  const [agents, setAgents] = useState<AgentEntry[]>([]);
  const [open, setOpen] = useState(true);

  useEffect(() => {
    getAgents()
      .then(setAgents)
      .catch(() => {
        // Non-critical — sidebar still works without agent list
      });
  }, []);

  return (
    <aside className="fixed top-0 left-0 h-screen w-60 bg-gray-900 flex flex-col border-r border-gray-800">
      {/* Logo */}
      <div className="flex items-center gap-2 px-5 py-5 border-b border-gray-800">
        <div className="h-8 w-8 rounded-lg bg-blue-600 flex items-center justify-center text-white font-bold text-sm">
          ZC
        </div>
        <span className="text-lg font-semibold text-white tracking-wide">
          ZeroClaw
        </span>
      </div>

      {/* Navigation */}
      <nav className="flex-1 overflow-y-auto py-4 px-3 space-y-1">
        {/* Dashboard */}
        <NavLink
          to="/"
          end
          className={({ isActive }) => navLinkClass(isActive)}
        >
          <LayoutDashboard className="h-5 w-5 flex-shrink-0" />
          <span>{t('nav.dashboard')}</span>
        </NavLink>

        {/* Chat section — default agent + delegate agents */}
        <div>
          {/* Default agent link + expand toggle */}
          <div className="flex items-center">
            <NavLink
              to="/agent"
              end
              className={({ isActive }) =>
                [
                  'flex-1 flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm font-medium transition-colors',
                  isActive
                    ? 'bg-blue-600 text-white'
                    : 'text-gray-300 hover:bg-gray-800 hover:text-white',
                ].join(' ')
              }
            >
              <MessageSquare className="h-5 w-5 flex-shrink-0" />
              <span>{t('nav.agent')}</span>
            </NavLink>
            {agents.length > 0 && (
              <button
                onClick={() => setOpen((v) => !v)}
                className="p-1.5 rounded text-gray-500 hover:text-gray-300 hover:bg-gray-800 transition-colors"
                aria-label={open ? 'Collapse agents' : 'Expand agents'}
              >
                {open ? (
                  <ChevronDown className="h-4 w-4" />
                ) : (
                  <ChevronRight className="h-4 w-4" />
                )}
              </button>
            )}
          </div>

          {/* Delegate agent list */}
          {open && agents.length > 0 && (
            <div className="mt-1 ml-3 space-y-0.5 border-l border-gray-800 pl-3">
              {agents.map((a) => {
                const label = a.name ?? a.id;
                const emoji = a.emoji;
                return (
                  <NavLink
                    key={a.id}
                    to={`/agent/${a.id}`}
                    className={({ isActive }) =>
                      [
                        'flex items-center gap-2 px-2 py-2 rounded-lg text-sm transition-colors',
                        isActive
                          ? 'bg-blue-600 text-white font-medium'
                          : 'text-gray-400 hover:bg-gray-800 hover:text-white',
                      ].join(' ')
                    }
                  >
                    {emoji ? (
                      <span className="text-base leading-none">{emoji}</span>
                    ) : (
                      <Bot className="h-4 w-4 flex-shrink-0" />
                    )}
                    <span className="truncate">{label}</span>
                  </NavLink>
                );
              })}
            </div>
          )}
        </div>

        {/* Rest of nav */}
        {bottomNavItems.map(({ to, icon: Icon, labelKey }) => (
          <NavLink
            key={to}
            to={to}
            end={to === '/'}
            className={({ isActive }) => navLinkClass(isActive)}
          >
            <Icon className="h-5 w-5 flex-shrink-0" />
            <span>{t(labelKey)}</span>
          </NavLink>
        ))}
      </nav>
    </aside>
  );
}
