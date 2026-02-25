import React, { useState, useEffect, useRef } from "react";
import { motion } from "framer-motion";
import { Button } from "@/components/ui/button";
import { api, type Agent } from "@/lib/api";
import {
  X,
  Bot,
  Search,
  Loader2,
  Users
} from "lucide-react";
import { cn } from "@/lib/utils";

interface AgentPickerProps {
  /**
   * The project ID to get agents for
   */
  projectId: string;
  /**
   * Callback when an agent is selected
   */
  onSelect: (agent: Agent) => void;
  /**
   * Callback to close the picker
   */
  onClose: () => void;
  /**
   * Initial search query
   */
  initialQuery?: string;
  /**
   * Optional className for styling
   */
  className?: string;
}

/**
 * AgentPicker component - Agent selector for @ mentions in chat
 *
 * @example
 * <AgentPicker
 *   projectId="project-123"
 *   onSelect={(agent) => console.log('Selected:', agent)}
 *   onClose={() => setShowPicker(false)}
 * />
 */
export const AgentPicker: React.FC<AgentPickerProps> = ({
  projectId,
  onSelect,
  onClose,
  initialQuery = "",
  className,
}) => {
  const [agents, setAgents] = useState<Agent[]>([]);
  const [filteredAgents, setFilteredAgents] = useState<Agent[]>([]);
  const [searchQuery, setSearchQuery] = useState(initialQuery);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedIndex, setSelectedIndex] = useState(0);

  const agentListRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Load agents on mount
  useEffect(() => {
    loadAgents();
  }, [projectId]);

  // Focus input on mount
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Filter agents when search query changes
  useEffect(() => {
    if (!searchQuery.trim()) {
      setFilteredAgents(agents);
    } else {
      const query = searchQuery.toLowerCase();
      setFilteredAgents(
        agents.filter(
          (agent) =>
            agent.name.toLowerCase().includes(query) ||
            agent.nickname?.toLowerCase().includes(query) ||
            agent.agent_type.toLowerCase().includes(query)
        )
      );
    }
    setSelectedIndex(0);
  }, [searchQuery, agents]);

  // Reset selected index when filtered agents change
  useEffect(() => {
    setSelectedIndex(0);
  }, [filteredAgents]);

  const loadAgents = async () => {
    try {
      setIsLoading(true);
      setError(null);
      const projectAgents = await api.listProjectAgents(projectId);
      setAgents(projectAgents);
      setFilteredAgents(projectAgents);
    } catch (err) {
      console.error("Failed to load project agents:", err);
      setError("Failed to load agents");
    } finally {
      setIsLoading(false);
    }
  };

  // Scroll selected item into view
  useEffect(() => {
    if (agentListRef.current) {
      const selectedElement = agentListRef.current.querySelector(`[data-index="${selectedIndex}"]`);
      if (selectedElement) {
        selectedElement.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
      }
    }
  }, [selectedIndex]);

  // Keyboard navigation
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      switch (e.key) {
        case 'Escape':
          e.preventDefault();
          onClose();
          break;

        case 'Enter':
          e.preventDefault();
          if (filteredAgents.length > 0 && selectedIndex < filteredAgents.length) {
            onSelect(filteredAgents[selectedIndex]);
          }
          break;

        case 'ArrowUp':
          e.preventDefault();
          setSelectedIndex((prev) => Math.max(0, prev - 1));
          break;

        case 'ArrowDown':
          e.preventDefault();
          setSelectedIndex((prev) => Math.min(filteredAgents.length - 1, prev + 1));
          break;
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [filteredAgents, selectedIndex, onClose, onSelect]);

  // Render agent icon
  const renderAgentIcon = (agent: Agent) => {
    // Try to use the agent's icon if available
    if (agent.icon) {
      return <span className="text-lg">{agent.icon}</span>;
    }
    return <Bot className="h-4 w-4" />;
  };

  // Get display name (prefer nickname over name)
  const getDisplayName = (agent: Agent) => {
    return agent.nickname || agent.name;
  };

  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: 8 }}
      transition={{ duration: 0.15 }}
      className={cn(
        "absolute bottom-full left-0 mb-2 w-80 bg-background border border-border rounded-lg shadow-lg overflow-hidden z-50",
        className
      )}
    >
      {/* Header */}
      <div className="flex items-center justify-between p-3 border-b bg-muted/50">
        <div className="flex items-center gap-2 text-sm font-medium">
          <Users className="h-4 w-4" />
          <span>Select Agent</span>
        </div>
        <Button
          variant="ghost"
          size="icon"
          onClick={onClose}
          className="h-6 w-6"
        >
          <X className="h-3.5 w-3.5" />
        </Button>
      </div>

      {/* Search Input */}
      <div className="p-2 border-b">
        <div className="relative">
          <Search className="absolute left-2.5 top-1/2 transform -translate-y-1/2 h-4 w-4 text-muted-foreground" />
          <input
            ref={inputRef}
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="Search agents..."
            className="w-full pl-9 pr-3 py-2 text-sm bg-muted border border-border rounded-md focus:outline-none focus:ring-2 focus:ring-primary"
          />
        </div>
      </div>

      {/* Agent List */}
      <div
        ref={agentListRef}
        className="max-h-60 overflow-y-auto p-1"
      >
        {isLoading ? (
          <div className="flex items-center justify-center py-8">
            <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
          </div>
        ) : error ? (
          <div className="flex items-center justify-center py-8 text-sm text-destructive">
            {error}
          </div>
        ) : filteredAgents.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-8 text-sm text-muted-foreground">
            <Bot className="h-8 w-8 mb-2 opacity-50" />
            <p>{searchQuery ? "No agents found" : "No agents in this project"}</p>
          </div>
        ) : (
          filteredAgents.map((agent, index) => (
            <button
              key={agent.id}
              data-index={index}
              onClick={() => onSelect(agent)}
              className={cn(
                "w-full flex items-center gap-3 p-2.5 rounded-md transition-colors text-left",
                index === selectedIndex
                  ? "bg-primary/10 text-primary"
                  : "hover:bg-muted"
              )}
            >
              <div className="flex-shrink-0 w-8 h-8 flex items-center justify-center rounded-full bg-muted">
                {renderAgentIcon(agent)}
              </div>
              <div className="flex-1 min-w-0">
                <div className="font-medium text-sm truncate">
                  {getDisplayName(agent)}
                </div>
                <div className="text-xs text-muted-foreground truncate">
                  {agent.agent_type}
                </div>
              </div>
              {agent.color && (
                <div
                  className="w-2 h-2 rounded-full"
                  style={{ backgroundColor: agent.color }}
                />
              )}
            </button>
          ))
        )}
      </div>

      {/* Footer hint */}
      <div className="p-2 border-t bg-muted/30">
        <div className="text-xs text-muted-foreground text-center">
          Use <kbd className="px-1 py-0.5 bg-muted rounded text-[10px]">↑</kbd>
          <kbd className="px-1 py-0.5 bg-muted rounded text-[10px] ml-0.5">↓</kbd>
          to navigate, <kbd className="px-1 py-0.5 bg-muted rounded text-[10px]">Enter</kbd> to select
        </div>
      </div>
    </motion.div>
  );
};
