import React, { useState, useRef, useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  Send,
  Maximize2,
  Minimize2,
  ChevronUp,
  Sparkles,
  Zap,
  Square,
  Brain,
  Lightbulb,
  Cpu,
  Rocket,
  
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Popover } from "@/components/ui/popover";
import { Textarea } from "@/components/ui/textarea";
import { TooltipSimple, Tooltip, TooltipTrigger, TooltipContent } from "@/components/ui/tooltip-modern";
import { FilePicker } from "./FilePicker";
import { SlashCommandPicker } from "./SlashCommandPicker";
import { AgentPicker } from "./AgentPicker";
import { ImagePreview } from "./ImagePreview";
import { type FileEntry, type SlashCommand, type Agent } from "@/lib/api";

// Conditional import for Tauri webview window
let tauriGetCurrentWebviewWindow: any;
try {
  if (typeof window !== 'undefined' && window.__TAURI__) {
    tauriGetCurrentWebviewWindow = require("@tauri-apps/api/webviewWindow").getCurrentWebviewWindow;
  }
} catch (e) {
  console.log('[FloatingPromptInput] Tauri webview API not available, using web mode');
}

// Web-compatible replacement
const getCurrentWebviewWindow = tauriGetCurrentWebviewWindow || (() => ({ listen: () => Promise.resolve(() => {}) }));

interface FloatingPromptInputProps {
  /**
   * Callback when prompt is sent
   */
  onSend: (prompt: string, model: "sonnet" | "opus") => void;
  /**
   * Whether the input is loading
   */
  isLoading?: boolean;
  /**
   * Whether the input is disabled
   */
  disabled?: boolean;
  /**
   * Default model to select
   */
  defaultModel?: "sonnet" | "opus";
  /**
   * Project path for file picker
   */
  projectPath?: string;
  /**
   * Project ID for message dispatching (used with onSendToAgent)
   */
  projectId?: string;
  /**
   * Callback for sending messages to agents via @ mention
   * If provided, messages with @username will be dispatched to the agent
   */
  onSendToAgent?: (prompt: string) => Promise<void>;
  /**
   * Optional className for styling
   */
  className?: string;
  /**
   * Callback when cancel is clicked (only during loading)
   */
  onCancel?: () => void;
  /**
   * Extra menu items to display in the prompt bar
   */
  extraMenuItems?: React.ReactNode;
}

export interface FloatingPromptInputRef {
  addImage: (imagePath: string) => void;
}

/**
 * Thinking mode type definition
 */
type ThinkingMode = "auto" | "think" | "think_hard" | "think_harder" | "ultrathink";

/**
 * Thinking mode configuration
 */
type ThinkingModeConfig = {
  id: ThinkingMode;
  name: string;
  description: string;
  level: number; // 0-4 for visual indicator
  phrase?: string; // The phrase to append
  icon: React.ReactNode;
  color: string;
  shortName: string;
};

const THINKING_MODES: ThinkingModeConfig[] = [
  {
    id: "auto",
    name: "Auto",
    description: "Let Claude decide",
    level: 0,
    icon: <Sparkles className="h-3.5 w-3.5" />,
    color: "text-muted-foreground",
    shortName: "A"
  },
  {
    id: "think",
    name: "Think",
    description: "Basic reasoning",
    level: 1,
    phrase: "think",
    icon: <Lightbulb className="h-3.5 w-3.5" />,
    color: "text-primary",
    shortName: "T"
  },
  {
    id: "think_hard",
    name: "Think Hard",
    description: "Deeper analysis",
    level: 2,
    phrase: "think hard",
    icon: <Brain className="h-3.5 w-3.5" />,
    color: "text-primary",
    shortName: "T+"
  },
  {
    id: "think_harder",
    name: "Think Harder",
    description: "Extensive reasoning",
    level: 3,
    phrase: "think harder",
    icon: <Cpu className="h-3.5 w-3.5" />,
    color: "text-primary",
    shortName: "T++"
  },
  {
    id: "ultrathink",
    name: "Ultrathink",
    description: "Maximum computation",
    level: 4,
    phrase: "ultrathink",
    icon: <Rocket className="h-3.5 w-3.5" />,
    color: "text-primary",
    shortName: "Ultra"
  }
];

/**
 * ThinkingModeIndicator component - Shows visual indicator bars for thinking level
 */
const ThinkingModeIndicator: React.FC<{ level: number; color?: string }> = ({ level, color: _color }) => {
  const getBarColor = (barIndex: number) => {
    if (barIndex > level) return "bg-muted";
    return "bg-primary";
  };
  
  return (
    <div className="flex items-center gap-0.5">
      {[1, 2, 3, 4].map((i) => (
        <div
          key={i}
          className={cn(
            "w-1 h-3 rounded-full transition-all duration-200",
            getBarColor(i),
            i <= level && "shadow-sm"
          )}
        />
      ))}
    </div>
  );
};

type Model = {
  id: "sonnet" | "opus";
  name: string;
  description: string;
  icon: React.ReactNode;
  shortName: string;
  color: string;
};

const MODELS: Model[] = [
  {
    id: "sonnet",
    name: "Claude 4 Sonnet",
    description: "Faster, efficient for most tasks",
    icon: <Zap className="h-3.5 w-3.5" />,
    shortName: "S",
    color: "text-primary"
  },
  {
    id: "opus",
    name: "Claude 4 Opus",
    description: "More capable, better for complex tasks",
    icon: <Zap className="h-3.5 w-3.5" />,
    shortName: "O",
    color: "text-primary"
  }
];

/**
 * FloatingPromptInput component - Fixed position prompt input with model picker
 * 
 * @example
 * const promptRef = useRef<FloatingPromptInputRef>(null);
 * <FloatingPromptInput
 *   ref={promptRef}
 *   onSend={(prompt, model) => console.log('Send:', prompt, model)}
 *   isLoading={false}
 * />
 */
const FloatingPromptInputInner = (
  {
    onSend,
    isLoading = false,
    disabled = false,
    defaultModel = "sonnet",
    projectPath,
    projectId,
    projectId: _projectId,
    onSendToAgent,
    className,
    onCancel,
    extraMenuItems,
  }: FloatingPromptInputProps,
  ref: React.Ref<FloatingPromptInputRef>,
) => {
  const [prompt, setPrompt] = useState("");
  const [selectedModel, setSelectedModel] = useState<"sonnet" | "opus">(defaultModel);
  const [selectedThinkingMode, setSelectedThinkingMode] = useState<ThinkingMode>("auto");
  const [isExpanded, setIsExpanded] = useState(false);
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const [thinkingModePickerOpen, setThinkingModePickerOpen] = useState(false);
  const [showFilePicker, setShowFilePicker] = useState(false);
  const [filePickerQuery, setFilePickerQuery] = useState("");
  const [showSlashCommandPicker, setShowSlashCommandPicker] = useState(false);
  const [slashCommandQuery, setSlashCommandQuery] = useState("");
  const [showAgentPicker, setShowAgentPicker] = useState(false);
  const [agentPickerQuery, setAgentPickerQuery] = useState("");
  const [cursorPosition, setCursorPosition] = useState(0);
  const [embeddedImages, setEmbeddedImages] = useState<string[]>([]);
  const [dragActive, setDragActive] = useState(false);

  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const expandedTextareaRef = useRef<HTMLTextAreaElement>(null);
  const unlistenDragDropRef = useRef<(() => void) | null>(null);
  const [textareaHeight, setTextareaHeight] = useState<number>(48);
  const isIMEComposingRef = useRef(false);

  // Expose a method to add images programmatically
  React.useImperativeHandle(
    ref,
    () => ({
      addImage: (imagePath: string) => {
        setPrompt(currentPrompt => {
          const existingPaths = extractImagePaths(currentPrompt);
          if (existingPaths.includes(imagePath)) {
            return currentPrompt; // Image already added
          }

          // Wrap path in quotes if it contains spaces
          const mention = imagePath.includes(' ') ? `@"${imagePath}"` : `@${imagePath}`;
          const newPrompt = currentPrompt + (currentPrompt.endsWith(' ') || currentPrompt === '' ? '' : ' ') + mention + ' ';

          // Focus the textarea
          setTimeout(() => {
            const target = isExpanded ? expandedTextareaRef.current : textareaRef.current;
            target?.focus();
            target?.setSelectionRange(newPrompt.length, newPrompt.length);
          }, 0);

          return newPrompt;
        });
      }
    }),
    [isExpanded]
  );

  // Helper function to check if a file is an image
  const isImageFile = (path: string): boolean => {
    // Check if it's a data URL
    if (path.startsWith('data:image/')) {
      return true;
    }
    // Otherwise check file extension
    const ext = path.split('.').pop()?.toLowerCase();
    return ['png', 'jpg', 'jpeg', 'gif', 'svg', 'webp', 'ico', 'bmp'].includes(ext || '');
  };

  // Extract image paths from prompt text
  const extractImagePaths = (text: string): string[] => {
    console.log('[extractImagePaths] Input text length:', text.length);
    
    // Updated regex to handle both quoted and unquoted paths
    // Pattern 1: @"path with spaces or data URLs" - quoted paths
    // Pattern 2: @path - unquoted paths (continues until @ or end)
    const quotedRegex = /@"([^"]+)"/g;
    const unquotedRegex = /@([^@\n\s]+)/g;
    
    const pathsSet = new Set<string>(); // Use Set to ensure uniqueness
    
    // First, extract quoted paths (including data URLs)
    let matches = Array.from(text.matchAll(quotedRegex));
    console.log('[extractImagePaths] Quoted matches:', matches.length);
    
    for (const match of matches) {
      const path = match[1]; // No need to trim, quotes preserve exact path
      console.log('[extractImagePaths] Processing quoted path:', path.startsWith('data:') ? 'data URL' : path);
      
      // For data URLs, use as-is; for file paths, convert to absolute
      const fullPath = path.startsWith('data:') 
        ? path 
        : (path.startsWith('/') ? path : (projectPath ? `${projectPath}/${path}` : path));
      
      if (isImageFile(fullPath)) {
        pathsSet.add(fullPath);
      }
    }
    
    // Remove quoted mentions from text to avoid double-matching
    let textWithoutQuoted = text.replace(quotedRegex, '');
    
    // Then extract unquoted paths (typically file paths)
    matches = Array.from(textWithoutQuoted.matchAll(unquotedRegex));
    console.log('[extractImagePaths] Unquoted matches:', matches.length);
    
    for (const match of matches) {
      const path = match[1].trim();
      // Skip if it looks like a data URL fragment (shouldn't happen with proper quoting)
      if (path.includes('data:')) continue;
      
      console.log('[extractImagePaths] Processing unquoted path:', path);
      
      // Convert relative path to absolute if needed
      const fullPath = path.startsWith('/') ? path : (projectPath ? `${projectPath}/${path}` : path);
      
      if (isImageFile(fullPath)) {
        pathsSet.add(fullPath);
      }
    }

    const uniquePaths = Array.from(pathsSet);
    console.log('[extractImagePaths] Final extracted paths (unique):', uniquePaths.length);
    return uniquePaths;
  };

  // Update embedded images when prompt changes
  useEffect(() => {
    console.log('[useEffect] Prompt changed:', prompt);
    const imagePaths = extractImagePaths(prompt);
    console.log('[useEffect] Setting embeddedImages to:', imagePaths);
    setEmbeddedImages(imagePaths);

    // Auto-resize is disabled - height is now controlled by h-full CSS class
    // This effect still runs to keep embeddedImages in sync
  }, [prompt, projectPath, isExpanded]);

  // Set up Tauri drag-drop event listener
  useEffect(() => {
    // This effect runs only once on component mount to set up the listener.
    let lastDropTime = 0;
    let unlistenDrop: (() => void) | null = null;
    let unlistenEnter: (() => void) | null = null;
    let unlistenLeave: (() => void) | null = null;

    const setupListener = async () => {
      try {
        // If listeners from a previous mount/render are still around, clean them up.
        if (unlistenDragDropRef.current) {
          unlistenDragDropRef.current();
        }

        const webview = getCurrentWebviewWindow();

        // Listen for drag enter/over events
        const handleDragEnterOver = () => setDragActive(true);
        const handleDragLeave = () => setDragActive(false);

        // Listen for drop event
        const handleDrop = async (event: { payload: { paths: string[] } }) => {
          setDragActive(false);

          const currentTime = Date.now();
          if (currentTime - lastDropTime < 200) {
            return;
          }
          lastDropTime = currentTime;

          const droppedPaths = event.payload.paths;
          const imagePaths = droppedPaths.filter(isImageFile);

          if (imagePaths.length > 0) {
            setPrompt(currentPrompt => {
              const existingPaths = extractImagePaths(currentPrompt);
              const newPaths = imagePaths.filter(p => !existingPaths.includes(p));

              if (newPaths.length === 0) {
                return currentPrompt;
              }

              const mentionsToAdd = newPaths.map(p => {
                if (p.includes(' ')) {
                  return `@"${p}"`;
                }
                return `@${p}`;
              }).join(' ');
              const newPrompt = currentPrompt + (currentPrompt.endsWith(' ') || currentPrompt === '' ? '' : ' ') + mentionsToAdd + ' ';

              setTimeout(() => {
                const target = isExpanded ? expandedTextareaRef.current : textareaRef.current;
                target?.focus();
                target?.setSelectionRange(newPrompt.length, newPrompt.length);
              }, 0);

              return newPrompt;
            });
          }
        };

        // Set up listeners using Tauri 2's listen API
        unlistenEnter = await webview.listen('tauri://drag-enter', handleDragEnterOver);
        unlistenEnter = await webview.listen('tauri://drag-over', handleDragEnterOver);
        unlistenLeave = await webview.listen('tauri://drag-leave', handleDragLeave);
        unlistenDrop = await webview.listen<{ paths: string[] }>('tauri://drop', handleDrop);

        // Store cleanup function
        unlistenDragDropRef.current = () => {
          unlistenDrop?.();
          unlistenEnter?.();
          unlistenLeave?.();
        };
      } catch (error) {
        console.error('Failed to set up Tauri drag-drop listener:', error);
      }
    };

    setupListener();

    return () => {
      // On unmount, ensure we clean up the listener.
      if (unlistenDragDropRef.current) {
        unlistenDragDropRef.current();
        unlistenDragDropRef.current = null;
      }
    };
  }, []); // Empty dependency array ensures this runs only on mount/unmount.

  useEffect(() => {
    // Focus the appropriate textarea when expanded state changes
    if (isExpanded && expandedTextareaRef.current) {
      expandedTextareaRef.current.focus();
    } else if (!isExpanded && textareaRef.current) {
      textareaRef.current.focus();
    }
  }, [isExpanded]);

  const handleTextChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const newValue = e.target.value;
    const newCursorPosition = e.target.selectionStart || 0;

    // Height is now controlled by h-full CSS class, no auto-resize needed

    // Check if / was just typed at the beginning of input or after whitespace
    if (newValue.length > prompt.length && newValue[newCursorPosition - 1] === '/') {
      // Check if it's at the start or after whitespace
      const isStartOfCommand = newCursorPosition === 1 || 
        (newCursorPosition > 1 && /\s/.test(newValue[newCursorPosition - 2]));
      
      if (isStartOfCommand) {
        console.log('[FloatingPromptInput] / detected for slash command');
        setShowSlashCommandPicker(true);
        setSlashCommandQuery("");
        setCursorPosition(newCursorPosition);
      }
    }

    // Check if @ was just typed
    if (newValue.length > prompt.length && newValue[newCursorPosition - 1] === '@') {
      console.log('[FloatingPromptInput] @ detected, projectId:', projectId, 'projectPath:', projectPath);
      // If projectId exists, show AgentPicker for member selection
      // Otherwise, show FilePicker for file selection (fallback)
      if (projectId?.trim()) {
        setShowAgentPicker(true);
        setAgentPickerQuery("");
      } else if (projectPath?.trim()) {
        setShowFilePicker(true);
        setFilePickerQuery("");
      }
      setCursorPosition(newCursorPosition);
    }

    // Check if we're typing after / (for slash command search)
    if (showSlashCommandPicker && newCursorPosition >= cursorPosition) {
      // Find the / position before cursor
      let slashPosition = -1;
      for (let i = newCursorPosition - 1; i >= 0; i--) {
        if (newValue[i] === '/') {
          slashPosition = i;
          break;
        }
        // Stop if we hit whitespace (new word)
        if (newValue[i] === ' ' || newValue[i] === '\n') {
          break;
        }
      }

      if (slashPosition !== -1) {
        const query = newValue.substring(slashPosition + 1, newCursorPosition);
        setSlashCommandQuery(query);
      } else {
        // / was removed or cursor moved away
        setShowSlashCommandPicker(false);
        setSlashCommandQuery("");
      }
    }

    // Check if we're typing after @ (for search query)
    if (showAgentPicker && newCursorPosition >= cursorPosition) {
      // Find the @ position before cursor
      let atPosition = -1;
      for (let i = newCursorPosition - 1; i >= 0; i--) {
        if (newValue[i] === '@') {
          atPosition = i;
          break;
        }
        // Stop if we hit whitespace (new word)
        if (newValue[i] === ' ' || newValue[i] === '\n') {
          break;
        }
      }

      if (atPosition !== -1) {
        const query = newValue.substring(atPosition + 1, newCursorPosition);
        setAgentPickerQuery(query);
      } else {
        // @ was removed or cursor moved away
        setShowAgentPicker(false);
        setAgentPickerQuery("");
      }
    } else if (showFilePicker && newCursorPosition >= cursorPosition) {
      // Find the @ position before cursor
      let atPosition = -1;
      for (let i = newCursorPosition - 1; i >= 0; i--) {
        if (newValue[i] === '@') {
          atPosition = i;
          break;
        }
        // Stop if we hit whitespace (new word)
        if (newValue[i] === ' ' || newValue[i] === '\n') {
          break;
        }
      }

      if (atPosition !== -1) {
        const query = newValue.substring(atPosition + 1, newCursorPosition);
        setFilePickerQuery(query);
      } else {
        // @ was removed or cursor moved away
        setShowFilePicker(false);
        setFilePickerQuery("");
      }
    }

    setPrompt(newValue);
    setCursorPosition(newCursorPosition);
  };

  const handleFileSelect = (entry: FileEntry) => {
    if (textareaRef.current) {
      // Find the @ position before cursor
      let atPosition = -1;
      for (let i = cursorPosition - 1; i >= 0; i--) {
        if (prompt[i] === '@') {
          atPosition = i;
          break;
        }
        // Stop if we hit whitespace (new word)
        if (prompt[i] === ' ' || prompt[i] === '\n') {
          break;
        }
      }

      if (atPosition === -1) {
        // @ not found, this shouldn't happen but handle gracefully
        console.error('[FloatingPromptInput] @ position not found');
        return;
      }

      // Replace the @ and partial query with the selected path (file or directory)
      const textarea = textareaRef.current;
      const beforeAt = prompt.substring(0, atPosition);
      const afterCursor = prompt.substring(cursorPosition);
      const relativePath = entry.path.startsWith(projectPath || '')
        ? entry.path.slice((projectPath || '').length + 1)
        : entry.path;

      const newPrompt = `${beforeAt}@${relativePath} ${afterCursor}`;
      setPrompt(newPrompt);
      setShowFilePicker(false);
      setFilePickerQuery("");

      // Focus back on textarea and set cursor position after the inserted path
      setTimeout(() => {
        textarea.focus();
        const newCursorPos = beforeAt.length + relativePath.length + 2; // +2 for @ and space
        textarea.setSelectionRange(newCursorPos, newCursorPos);
      }, 0);
    }
  };

  const handleFilePickerClose = () => {
    setShowFilePicker(false);
    setFilePickerQuery("");
    // Return focus to textarea
    setTimeout(() => {
      textareaRef.current?.focus();
    }, 0);
  };

  const handleAgentSelect = (agent: Agent) => {
    const textarea = isExpanded ? expandedTextareaRef.current : textareaRef.current;
    if (!textarea) return;

    // Find the @ position before cursor
    let atPosition = -1;
    for (let i = cursorPosition - 1; i >= 0; i--) {
      if (prompt[i] === '@') {
        atPosition = i;
        break;
      }
      // Stop if we hit whitespace (new word)
      if (prompt[i] === ' ' || prompt[i] === '\n') {
        break;
      }
    }

    if (atPosition === -1) {
      console.error('[FloatingPromptInput] @ position not found');
      return;
    }

    // Replace the @ and partial query with the selected agent mention
    const textareaEl = textarea;
    const beforeAt = prompt.substring(0, atPosition);
    const afterCursor = prompt.substring(cursorPosition);

    // Use nickname or name for display
    const agentMention = agent.nickname || agent.name;
    const newPrompt = `${beforeAt}@${agentMention} ${afterCursor}`;
    setPrompt(newPrompt);
    setShowAgentPicker(false);
    setAgentPickerQuery("");

    // Focus back on textarea and set cursor position after the agent mention
    setTimeout(() => {
      textareaEl.focus();
      const newCursorPos = beforeAt.length + agentMention.length + 2; // +2 for @ and space
      textareaEl.setSelectionRange(newCursorPos, newCursorPos);
    }, 0);
  };

  const handleAgentPickerClose = () => {
    setShowAgentPicker(false);
    setAgentPickerQuery("");
    // Return focus to textarea
    setTimeout(() => {
      textareaRef.current?.focus();
    }, 0);
  };

  const handleSlashCommandSelect = (command: SlashCommand) => {
    const textarea = isExpanded ? expandedTextareaRef.current : textareaRef.current;
    if (!textarea) return;

    // Find the / position before cursor
    let slashPosition = -1;
    for (let i = cursorPosition - 1; i >= 0; i--) {
      if (prompt[i] === '/') {
        slashPosition = i;
        break;
      }
      // Stop if we hit whitespace (new word)
      if (prompt[i] === ' ' || prompt[i] === '\n') {
        break;
      }
    }

    if (slashPosition === -1) {
      console.error('[FloatingPromptInput] / position not found');
      return;
    }

    // Simply insert the command syntax
    const beforeSlash = prompt.substring(0, slashPosition);
    const afterCursor = prompt.substring(cursorPosition);
    
    if (command.accepts_arguments) {
      // Insert command with placeholder for arguments
      const newPrompt = `${beforeSlash}${command.full_command} `;
      setPrompt(newPrompt);
      setShowSlashCommandPicker(false);
      setSlashCommandQuery("");

      // Focus and position cursor after the command
      setTimeout(() => {
        textarea.focus();
        const newCursorPos = beforeSlash.length + command.full_command.length + 1;
        textarea.setSelectionRange(newCursorPos, newCursorPos);
      }, 0);
    } else {
      // Insert command and close picker
      const newPrompt = `${beforeSlash}${command.full_command} ${afterCursor}`;
      setPrompt(newPrompt);
      setShowSlashCommandPicker(false);
      setSlashCommandQuery("");

      // Focus and position cursor after the command
      setTimeout(() => {
        textarea.focus();
        const newCursorPos = beforeSlash.length + command.full_command.length + 1;
        textarea.setSelectionRange(newCursorPos, newCursorPos);
      }, 0);
    }
  };

  const handleSlashCommandPickerClose = () => {
    setShowSlashCommandPicker(false);
    setSlashCommandQuery("");
    // Return focus to textarea
    setTimeout(() => {
      const textarea = isExpanded ? expandedTextareaRef.current : textareaRef.current;
      textarea?.focus();
    }, 0);
  };

  const handleCompositionStart = () => {
    isIMEComposingRef.current = true;
  };

  const handleCompositionEnd = () => {
    setTimeout(() => {
      isIMEComposingRef.current = false;
    }, 0);
  };

  const isIMEInteraction = (event?: React.KeyboardEvent) => {
    if (isIMEComposingRef.current) {
      return true;
    }

    if (!event) {
      return false;
    }

    const nativeEvent = event.nativeEvent;

    if (nativeEvent.isComposing) {
      return true;
    }

    const key = nativeEvent.key;
    if (key === 'Process' || key === 'Unidentified') {
      return true;
    }

    const keyboardEvent = nativeEvent as unknown as KeyboardEvent;
    const keyCode = keyboardEvent.keyCode ?? (keyboardEvent as unknown as { which?: number }).which;
    if (keyCode === 229) {
      return true;
    }

    return false;
  };

  // Regex to match @username pattern
  const AT_MENTION_REGEX = /@(\w+)/;

  const handleSend = async () => {
    if (isIMEInteraction()) {
      return;
    }

    if (prompt.trim() && !disabled) {
      let finalPrompt = prompt.trim();

      // Check if prompt contains @ mention and onSendToAgent is provided
      if (onSendToAgent && AT_MENTION_REGEX.test(finalPrompt)) {
        // Send to agent via @ mention
        try {
          await onSendToAgent(finalPrompt);
        } catch (error) {
          console.error('[FloatingPromptInput] Failed to send to agent:', error);
          // Fall through to normal send
        }
        setPrompt("");
        setEmbeddedImages([]);
        // Height is now controlled by h-full, no need to reset
        return;
      }

      // Append thinking phrase if not auto mode
      const thinkingMode = THINKING_MODES.find(m => m.id === selectedThinkingMode);
      if (thinkingMode && thinkingMode.phrase) {
        finalPrompt = `${finalPrompt}.\n\n${thinkingMode.phrase}.`;
      }

      onSend(finalPrompt, selectedModel);
      setPrompt("");
      setEmbeddedImages([]);
      // Height is controlled by h-full, no reset needed
    }
  };

  const handleKeyDown = async (e: React.KeyboardEvent) => {
    if (showAgentPicker && e.key === 'Escape') {
      e.preventDefault();
      setShowAgentPicker(false);
      setAgentPickerQuery("");
      return;
    }

    if (showFilePicker && e.key === 'Escape') {
      e.preventDefault();
      setShowFilePicker(false);
      setFilePickerQuery("");
      return;
    }

    if (showSlashCommandPicker && e.key === 'Escape') {
      e.preventDefault();
      setShowSlashCommandPicker(false);
      setSlashCommandQuery("");
      return;
    }

    // Add keyboard shortcut for expanding
    if (e.key === 'e' && (e.ctrlKey || e.metaKey) && e.shiftKey) {
      e.preventDefault();
      setIsExpanded(true);
      return;
    }

    if (
      e.key === "Enter" &&
      !e.shiftKey &&
      !isExpanded &&
      !showFilePicker &&
      !showSlashCommandPicker
    ) {
      if (isIMEInteraction(e)) {
        return;
      }
      e.preventDefault();
      await handleSend();
    }
  };

  const handlePaste = async (e: React.ClipboardEvent) => {
    const items = e.clipboardData?.items;
    if (!items) return;

    for (const item of items) {
      if (item.type.startsWith('image/')) {
        e.preventDefault();
        
        // Get the image blob
        const blob = item.getAsFile();
        if (!blob) continue;

        try {
          // Convert blob to base64
          const reader = new FileReader();
          reader.onload = () => {
            const base64Data = reader.result as string;
            
            // Add the base64 data URL directly to the prompt
            setPrompt(currentPrompt => {
              // Use the data URL directly as the image reference
              const mention = `@"${base64Data}"`;
              const newPrompt = currentPrompt + (currentPrompt.endsWith(' ') || currentPrompt === '' ? '' : ' ') + mention + ' ';
              
              // Focus the textarea and move cursor to end
              setTimeout(() => {
                const target = isExpanded ? expandedTextareaRef.current : textareaRef.current;
                target?.focus();
                target?.setSelectionRange(newPrompt.length, newPrompt.length);
              }, 0);

              return newPrompt;
            });
          };
          
          reader.readAsDataURL(blob);
        } catch (error) {
          console.error('Failed to paste image:', error);
        }
      }
    }
  };

  // Browser drag and drop handlers - just prevent default behavior
  // Actual file handling is done via Tauri's window-level drag-drop events
  const handleDrag = (e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    // Visual feedback is handled by Tauri events
  };

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    // File processing is handled by Tauri's onDragDropEvent
  };

  const handleRemoveImage = (index: number) => {
    // Remove the corresponding @mention from the prompt
    const imagePath = embeddedImages[index];
    
    // For data URLs, we need to handle them specially since they're always quoted
    if (imagePath.startsWith('data:')) {
      // Simply remove the exact quoted data URL
      const quotedPath = `@"${imagePath}"`;
      const newPrompt = prompt.replace(quotedPath, '').trim();
      setPrompt(newPrompt);
      return;
    }
    
    // For file paths, use the original logic
    const escapedPath = imagePath.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const escapedRelativePath = imagePath.replace(projectPath + '/', '').replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    
    // Create patterns for both quoted and unquoted mentions
    const patterns = [
      // Quoted full path
      new RegExp(`@"${escapedPath}"\\s?`, 'g'),
      // Unquoted full path
      new RegExp(`@${escapedPath}\\s?`, 'g'),
      // Quoted relative path
      new RegExp(`@"${escapedRelativePath}"\\s?`, 'g'),
      // Unquoted relative path
      new RegExp(`@${escapedRelativePath}\\s?`, 'g')
    ];

    let newPrompt = prompt;
    for (const pattern of patterns) {
      newPrompt = newPrompt.replace(pattern, '');
    }

    setPrompt(newPrompt.trim());
  };

  const selectedModelData = MODELS.find(m => m.id === selectedModel) || MODELS[0];

  return (
    <>
      {/* Expanded Modal */}
      <AnimatePresence>
        {isExpanded && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-background/80 backdrop-blur-sm"
            onClick={() => setIsExpanded(false)}
          >
            <motion.div
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -8 }}
              transition={{ duration: 0.15 }}
              className="bg-background border border-border rounded-lg shadow-lg w-full max-w-2xl p-4 space-y-4"
              onClick={(e) => e.stopPropagation()}
            >
              <div className="flex items-center justify-between">
                <h3 className="text-sm font-medium">Compose your prompt</h3>
                <TooltipSimple content="Minimize" side="bottom">
                  <motion.div
                    whileTap={{ scale: 0.97 }}
                    transition={{ duration: 0.15 }}
                  >
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => setIsExpanded(false)}
                      className="h-8 w-8"
                    >
                      <Minimize2 className="h-4 w-4" />
                    </Button>
                  </motion.div>
                </TooltipSimple>
              </div>

              {/* Image previews in expanded mode */}
              {embeddedImages.length > 0 && (
                <ImagePreview
                  images={embeddedImages}
                  onRemove={handleRemoveImage}
                  className="border-t border-border pt-2"
                />
              )}

              <Textarea
                ref={expandedTextareaRef}
                value={prompt}
                onChange={handleTextChange}
                onCompositionStart={handleCompositionStart}
                onCompositionEnd={handleCompositionEnd}
                onPaste={handlePaste}
                placeholder="Type your message..."
                className="min-h-[200px] resize-none"
                disabled={disabled}
                onDragEnter={handleDrag}
                onDragLeave={handleDrag}
                onDragOver={handleDrag}
                onDrop={handleDrop}
              />

              <div className="flex items-center justify-between">
                <div className="flex items-center gap-4">
                  <div className="flex items-center gap-2">
                    <span className="text-xs text-muted-foreground">Model:</span>
                    <Popover
                      trigger={
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => setModelPickerOpen(!modelPickerOpen)}
                          className="gap-2"
                        >
                          <span className={selectedModelData.color}>
                            {selectedModelData.icon}
                          </span>
                          {selectedModelData.name}
                        </Button>
                      }
                      content={
                        <div className="w-[300px] p-1">
                          {MODELS.map((model) => (
                            <button
                              key={model.id}
                              onClick={() => {
                                setSelectedModel(model.id);
                                setModelPickerOpen(false);
                              }}
                              className={cn(
                                "w-full flex items-start gap-3 p-3 rounded-md transition-colors text-left",
                                "hover:bg-accent",
                                selectedModel === model.id && "bg-accent"
                              )}
                            >
                              <div className="mt-0.5">
                                <span className={model.color}>
                                  {model.icon}
                                </span>
                              </div>
                              <div className="flex-1 space-y-1">
                                <div className="font-medium text-sm">{model.name}</div>
                                <div className="text-xs text-muted-foreground">
                                  {model.description}
                                </div>
                              </div>
                            </button>
                          ))}
                        </div>
                      }
                      open={modelPickerOpen}
                      onOpenChange={setModelPickerOpen}
                      align="start"
                      side="top"
                    />
                  </div>

                  <div className="flex items-center gap-2">
                    <span className="text-xs text-muted-foreground">Thinking:</span>
                    <Popover
                      trigger={
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <Button
                              variant="outline"
                                size="sm"
                                onClick={() => setThinkingModePickerOpen(!thinkingModePickerOpen)}
                                className="gap-2"
                              >
                                <span className={THINKING_MODES.find(m => m.id === selectedThinkingMode)?.color}>
                                  {THINKING_MODES.find(m => m.id === selectedThinkingMode)?.icon}
                                </span>
                                <ThinkingModeIndicator 
                                  level={THINKING_MODES.find(m => m.id === selectedThinkingMode)?.level || 0} 
                                />
                              </Button>
                            </TooltipTrigger>
                            <TooltipContent>
                              <p className="font-medium">{THINKING_MODES.find(m => m.id === selectedThinkingMode)?.name || "Auto"}</p>
                              <p className="text-xs text-muted-foreground">{THINKING_MODES.find(m => m.id === selectedThinkingMode)?.description}</p>
                            </TooltipContent>
                          </Tooltip>
                      }
                      content={
                        <div className="w-[280px] p-1">
                          {THINKING_MODES.map((mode) => (
                            <button
                              key={mode.id}
                              onClick={() => {
                                setSelectedThinkingMode(mode.id);
                                setThinkingModePickerOpen(false);
                              }}
                              className={cn(
                                "w-full flex items-start gap-3 p-3 rounded-md transition-colors text-left",
                                "hover:bg-accent",
                                selectedThinkingMode === mode.id && "bg-accent"
                              )}
                            >
                              <span className={cn("mt-0.5", mode.color)}>
                                {mode.icon}
                              </span>
                              <div className="flex-1 space-y-1">
                                <div className="font-medium text-sm">
                                  {mode.name}
                                </div>
                                <div className="text-xs text-muted-foreground">
                                  {mode.description}
                                </div>
                              </div>
                              <ThinkingModeIndicator level={mode.level} />
                            </button>
                          ))}
                        </div>
                      }
                      open={thinkingModePickerOpen}
                      onOpenChange={setThinkingModePickerOpen}
                      align="start"
                      side="top"
                    />
                  </div>
                </div>

                <TooltipSimple content="Send message" side="top">
                  <motion.div
                    whileTap={{ scale: 0.97 }}
                    transition={{ duration: 0.15 }}
                  >
                    <Button
                      onClick={() => handleSend()}
                      disabled={!prompt.trim() || disabled}
                      size="default"
                      className="min-w-[60px]"
                    >
                      {isLoading ? (
                        <div className="rotating-symbol text-primary-foreground" />
                      ) : (
                        <Send className="h-4 w-4" />
                      )}
                    </Button>
                  </motion.div>
                </TooltipSimple>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Fixed Position Input Bar */}
      <div
        className={cn(
          "bottom-0 left-0 right-0 z-40 backdrop-blur-sm border-t h-full",
          dragActive && "ring-2 ring-primary ring-offset-2",
          className
        )}
        onDragEnter={handleDrag}
        onDragLeave={handleDrag}
        onDragOver={handleDrag}
        onDrop={handleDrop}
      >
        <div className="container mx-auto h-full">
          {/* Image previews */}
          {embeddedImages.length > 0 && (
            <ImagePreview
              images={embeddedImages}
              onRemove={handleRemoveImage}
              className="border-b border-border"
            />
          )}

          <div className="p-3 h-full">
            <div className="flex items-end gap-2 h-full">
              {/* Model & Thinking Mode Selectors - Left side, fixed at bottom */}
              <div className="flex items-center gap-1 shrink-0 mb-1 hidden">
                <Popover
                  trigger={
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <motion.div
                          whileTap={{ scale: 0.97 }}
                            transition={{ duration: 0.15 }}
                          >
                            <Button
                              variant="ghost"
                              size="sm"
                              disabled={disabled}
                              className="h-9 px-2 hover:bg-accent/50 gap-1"
                            >
                              <span className={selectedModelData.color}>
                                {selectedModelData.icon}
                              </span>
                              <span className="text-[10px] font-bold opacity-70">
                                {selectedModelData.shortName}
                              </span>
                              <ChevronUp className="h-3 w-3 ml-0.5 opacity-50" />
                            </Button>
                          </motion.div>
                        </TooltipTrigger>
                        <TooltipContent side="top">
                          <p className="text-xs font-medium">{selectedModelData.name}</p>
                          <p className="text-xs text-muted-foreground">{selectedModelData.description}</p>
                        </TooltipContent>
                      </Tooltip>
                  }
                content={
                  <div className="w-[300px] p-1">
                    {MODELS.map((model) => (
                      <button
                        key={model.id}
                        onClick={() => {
                          setSelectedModel(model.id);
                          setModelPickerOpen(false);
                        }}
                        className={cn(
                          "w-full flex items-start gap-3 p-3 rounded-md transition-colors text-left",
                          "hover:bg-accent",
                          selectedModel === model.id && "bg-accent"
                        )}
                      >
                        <div className="mt-0.5">
                          <span className={model.color}>
                            {model.icon}
                          </span>
                        </div>
                        <div className="flex-1 space-y-1">
                          <div className="font-medium text-sm">{model.name}</div>
                          <div className="text-xs text-muted-foreground">
                            {model.description}
                          </div>
                        </div>
                      </button>
                    ))}
                  </div>
                }
                open={modelPickerOpen}
                onOpenChange={setModelPickerOpen}
                align="start"
                side="top"
              />

                <Popover
                  trigger={
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <motion.div
                          whileTap={{ scale: 0.97 }}
                            transition={{ duration: 0.15 }}
                          >
                            <Button
                              variant="ghost"
                              size="sm"
                              disabled={disabled}
                              className="h-9 px-2 hover:bg-accent/50 gap-1"
                            >
                              <span className={THINKING_MODES.find(m => m.id === selectedThinkingMode)?.color}>
                                {THINKING_MODES.find(m => m.id === selectedThinkingMode)?.icon}
                              </span>
                              <span className="text-[10px] font-semibold opacity-70">
                                {THINKING_MODES.find(m => m.id === selectedThinkingMode)?.shortName}
                              </span>
                              <ChevronUp className="h-3 w-3 ml-0.5 opacity-50" />
                            </Button>
                          </motion.div>
                        </TooltipTrigger>
                        <TooltipContent side="top">
                          <p className="text-xs font-medium">Thinking: {THINKING_MODES.find(m => m.id === selectedThinkingMode)?.name || "Auto"}</p>
                          <p className="text-xs text-muted-foreground">{THINKING_MODES.find(m => m.id === selectedThinkingMode)?.description}</p>
                        </TooltipContent>
                      </Tooltip>
                  }
                content={
                  <div className="w-[280px] p-1">
                    {THINKING_MODES.map((mode) => (
                      <button
                        key={mode.id}
                        onClick={() => {
                          setSelectedThinkingMode(mode.id);
                          setThinkingModePickerOpen(false);
                        }}
                        className={cn(
                          "w-full flex items-start gap-3 p-3 rounded-md transition-colors text-left",
                          "hover:bg-accent",
                          selectedThinkingMode === mode.id && "bg-accent"
                        )}
                      >
                        <span className={cn("mt-0.5", mode.color)}>
                          {mode.icon}
                        </span>
                        <div className="flex-1 space-y-1">
                          <div className="font-medium text-sm">
                            {mode.name}
                          </div>
                          <div className="text-xs text-muted-foreground">
                            {mode.description}
                          </div>
                        </div>
                        <ThinkingModeIndicator level={mode.level} />
                      </button>
                    ))}
                  </div>
                }
                open={thinkingModePickerOpen}
                onOpenChange={setThinkingModePickerOpen}
                align="start"
                side="top"
              />

              </div>

              {/* Prompt Input - Center */}
              <div className="flex-1 relative h-full">
                <Textarea
                  ref={textareaRef}
                  value={prompt}
                  onChange={handleTextChange}
                  onKeyDown={handleKeyDown}
                  onCompositionStart={handleCompositionStart}
                  onCompositionEnd={handleCompositionEnd}
                  onPaste={handlePaste}
                  placeholder={
                    dragActive
                      ? "Drop images here..."
                      : projectId
                        ? "Message Claude (@ for members, / for commands)..."
                        : "Message Claude (@ for files, / for commands)..."
                  }
                  disabled={disabled}
                  className={cn(
                    "resize-none pr-20 pl-3 py-2.5 transition-all duration-150 h-full bg-white border-none",
                    dragActive && "border-primary"
                  )}
                />

                {/* Action buttons inside input - fixed at bottom right */}
                <div className="absolute right-1.5 bottom-1.5 flex items-center gap-0.5">
                  <TooltipSimple content="Expand (Ctrl+Shift+E)" side="top">
                    <motion.div
                      whileTap={{ scale: 0.97 }}
                      transition={{ duration: 0.15 }}
                    >
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => setIsExpanded(true)}
                        disabled={disabled}
                        className="h-8 w-8 hover:bg-accent/50 transition-colors"
                      >
                        <Maximize2 className="h-3.5 w-3.5" />
                      </Button>
                    </motion.div>
                  </TooltipSimple>

                  <TooltipSimple content={isLoading ? "Stop generation" : "Send message (Enter)"} side="top">
                    <motion.div
                      whileTap={{ scale: 0.97 }}
                      transition={{ duration: 0.15 }}
                    >
                      <Button
                        onClick={() => isLoading ? onCancel?.() : handleSend()}
                        disabled={isLoading ? false : (!prompt.trim() || disabled)}
                        variant={isLoading ? "destructive" : prompt.trim() ? "default" : "ghost"}
                        size="icon"
                        className={cn(
                          "h-8 w-8 transition-all",
                          prompt.trim() && !isLoading && "shadow-sm"
                        )}
                      >
                        {isLoading ? (
                          <Square className="h-4 w-4" />
                        ) : (
                          <Send className="h-4 w-4" />
                        )}
                      </Button>
                    </motion.div>
                  </TooltipSimple>
                </div>

                {/* File Picker */}
                <AnimatePresence>
                  {showFilePicker && projectPath && projectPath.trim() && (
                    <FilePicker
                      basePath={projectPath.trim()}
                      onSelect={handleFileSelect}
                      onClose={handleFilePickerClose}
                      initialQuery={filePickerQuery}
                    />
                  )}
                </AnimatePresence>

                {/* Agent Picker for @ mentions */}
                <AnimatePresence>
                  {showAgentPicker && projectId && projectId.trim() && (
                    <AgentPicker
                      projectId={projectId.trim()}
                      onSelect={handleAgentSelect}
                      onClose={handleAgentPickerClose}
                      initialQuery={agentPickerQuery}
                    />
                  )}
                </AnimatePresence>

                {/* Slash Command Picker */}
                <AnimatePresence>
                  {showSlashCommandPicker && (
                    <SlashCommandPicker
                      projectPath={projectPath}
                      onSelect={handleSlashCommandSelect}
                      onClose={handleSlashCommandPickerClose}
                      initialQuery={slashCommandQuery}
                    />
                  )}
                </AnimatePresence>
              </div>

              {/* Extra menu items - Right side, fixed at bottom */}
              {extraMenuItems && (
                <div className="flex items-center gap-0.5 shrink-0 mb-1">
                  {extraMenuItems}
                </div>
              )}
            </div>
          </div>
        </div>
      </div>
    </>
  );
};

export const FloatingPromptInput = React.forwardRef<
  FloatingPromptInputRef,
  FloatingPromptInputProps
>(FloatingPromptInputInner);

FloatingPromptInput.displayName = 'FloatingPromptInput';
