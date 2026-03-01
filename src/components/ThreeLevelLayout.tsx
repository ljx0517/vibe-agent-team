import React, { useEffect, useState } from 'react';
import { motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  FolderOpen, FileText, Users, BarChart, MessageSquare, Settings,
  Search, Plus, MoreVertical, UserPlus, Smile, Scissors,
  Image, FileVideo, ListTodo, FolderPlus, MoreHorizontal,
  Zap, BookOpen
} from 'lucide-react';
import { cn } from '@/lib/utils';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Input } from '@/components/ui/input';
import { Loader2, Check } from 'lucide-react';
import { ICON_MAP } from "./IconPicker";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { CreateProjectDialog } from '@/components/CreateProjectDialog';
import { Settings as SettingsComponent } from '@/components/Settings';
import { FloatingPromptInput } from './FloatingPromptInput';
import { Teammates } from './Teammates';
import { api, type Agent, type Message } from '@/lib/api';
// 项目进度类型
interface ProjectProgress {
  step: string;
  message: string;
}

// 项目类型
interface ProjectInfo {
  project_id: string;
  project_name: string;
  workspace_id: string;
  workspace_path: string;
  initializing?: boolean;
  progress?: ProjectProgress | null;
}

// 成员类型
interface Member {
  id: string;
  name: string;
  avatar?: string;
  role_type?: string;
  status?: "pending" | "running" | "completed" | "stopped" | "error";
}

// 左侧导航项
interface NavItem {
  id: string;
  label: string;
  icon: React.ReactNode;
}

// 全局导航列表
const globalNavItems: NavItem[] = [
  { id: 'projects', label: '项目', icon: <FolderOpen className="w-5 h-5" /> },
  { id: 'documents', label: '文档', icon: <FileText className="w-5 h-5" /> },
  { id: 'team', label: '成员管理', icon: <Users className="w-5 h-5" /> },
  { id: 'analytics', label: '分析', icon: <BarChart className="w-5 h-5" /> },
  { id: 'messages', label: '消息', icon: <MessageSquare className="w-5 h-5" /> },
  { id: 'settings', label: '设置', icon: <Settings className="w-5 h-5" /> },
];

interface SelectedTeamlead {
  id: string;
  name: string;
  nickname?: string;
  gender?: string;
  prompt?: string;
  model?: string;
  color?: string;
}

interface ThreeLevelLayoutProps {
  className?: string;
  onAddClick?: (project: { name: string; projectCode?: string; description: string; workDir: string; teamlead?: SelectedTeamlead }) => void;
  projects?: ProjectInfo[];
  members?: Member[];
  isCreatingProject?: boolean;
  onCreatingProjectChange?: (isCreating: boolean) => void;
}

export const ThreeLevelLayout: React.FC<ThreeLevelLayoutProps> = ({
  className,
  onAddClick,
  projects = [],
  members: _members = [], // 使用 _members 避免未使用警告，内部使用 projectMembers
  isCreatingProject = false,
  onCreatingProjectChange,
}) => {
  const [selectedNav, setSelectedNav] = useState<string>('projects');
  const [selectedProject, setSelectedProject] = useState<ProjectInfo | null>(null);
  const [projectSearchQuery, setProjectSearchQuery] = useState('');
  const [showCreateDialog, setShowCreateDialog] = useState(false);
  const [projectMembers, setProjectMembers] = useState<Member[]>([]);
  const [dividerPosition, setDividerPosition] = useState(70); // 默认上部分70%
  const [isDragging, setIsDragging] = useState(false);
  const [isSending, setIsSending] = useState(false);
  const [showAddMemberModal, setShowAddMemberModal] = useState(false);
  const [availableMembers, setAvailableMembers] = useState<Agent[]>([]);
  const [loadingAvailableMembers, setLoadingAvailableMembers] = useState(false);
  const [selectedMemberIds, setSelectedMemberIds] = useState<string[]>([]);
  const [isAddingMembers, setIsAddingMembers] = useState(false);
  const [messages, setMessages] = useState<Message[]>([]);
  const [loadingMessages, setLoadingMessages] = useState(false);

  // 加载可选成员（排除 teamlead 角色，排除已添加到当前项目的成员）
  const loadAvailableMembers = async () => {
    try {
      setLoadingAvailableMembers(true);
      setSelectedMemberIds([]); // 重置选择
      console.log('[AddMember] Loading agents...');
      const allAgents = await api.listAgents();
      console.log('[AddMember] All agents with role_type:', allAgents.map(a => ({ name: a.name, role_type: a.role_type, id: a.id })));
      console.log('[AddMember] Current project members:', projectMembers.map(m => ({ id: m.id, name: m.name })));

      // 排除已添加到当前项目的成员
      const existingMemberIds = new Set(projectMembers.map(m => m.id));
      console.log('[AddMember] Existing member IDs:', existingMemberIds);

      const filtered = allAgents.filter(agent => {
        if (!agent.id) return false;
        // 排除 teamlead 角色
        if (agent.role_type === 'teamlead') return false;
        // 排除已添加的成员
        if (existingMemberIds.has(agent.id)) return false;
        return true;
      });
      console.log('[AddMember] Filtered agents:', filtered);
      setAvailableMembers(filtered);
    } catch (error) {
      console.error('[AddMember] Failed to load available members:', error);
    } finally {
      setLoadingAvailableMembers(false);
    }
  };

  // 点击添加成员按钮
  const handleAddMemberClick = () => {
    loadAvailableMembers();
    setShowAddMemberModal(true);
  };

  // 切换成员选择
  const toggleMemberSelection = (agentId: string) => {
    console.log('[AddMember] toggleMemberSelection called, agentId:', agentId);
    setSelectedMemberIds(prev => {
      const newIds = prev.includes(agentId)
        ? prev.filter(id => id !== agentId)
        : [...prev, agentId];
      console.log('[AddMember] selectedMemberIds changed:', prev, '->', newIds);
      return newIds;
    });
  };

  // 添加选中的成员到项目
  const handleAddSelectedMembers = async () => {
    if (!selectedProject || selectedMemberIds.length === 0) return;

    setIsAddingMembers(true);
    try {
      console.log('[AddMember] Adding members to project:', selectedProject.project_id, selectedMemberIds);
      // 批量添加到 project_agents 关联表
      await Promise.all(
        selectedMemberIds.map(memberId =>
          api.addAgentToProject(selectedProject.project_id, memberId)
        )
      );
      console.log('[AddMember] Successfully added members:', selectedMemberIds);
      // 刷新项目成员列表
      if (selectedProject.project_id) {
        const agents = await api.listProjectAgents(selectedProject.project_id);
        console.log('[AddMember] Refreshed project agents:', agents.map(a => ({ id: a.id, name: a.name, role_type: a.role_type })));
        const members: Member[] = agents.map(agent => ({
          id: String(agent.id) || '',
          name: agent.nickname || agent.name,
          avatar: agent.icon || undefined,
          role_type: agent.role_type,
        }));
        console.log('[AddMember] Setting project members:', members);
        setProjectMembers(members);
      }
      setShowAddMemberModal(false);
      setSelectedMemberIds([]);
    } catch (error) {
      console.error('[AddMember] Failed to add members:', error);
    } finally {
      setIsAddingMembers(false);
    }
  };

  // 发送消息处理函数
  const handleSendMessage = async (text: string, _model: "sonnet" | "opus") => {
    if (!selectedProject || !text.trim()) return;

    setIsSending(true);
    try {
      await api.sendMessage(selectedProject.project_id, text.trim());
      // 发送后立即刷新消息列表
      const msgs = await api.getMessages(selectedProject.project_id);
      setMessages(msgs);
    } catch (error) {
      console.error('Failed to send message:', error);
    } finally {
      setIsSending(false);
    }
  };

  useEffect(() => {
    console.log('current selectedNav', selectedNav)
  }, [selectedNav])

  // 当选中项目变化时，获取项目成员
  useEffect(() => {
    const fetchProjectMembers = async () => {
      if (selectedProject) {
        try {
          const agents = await invoke<Array<{
            id: string | null;
            name: string;
            icon: string;
            color: string | null;
            nickname: string | null;
            role_type?: string;
          }>>('list_project_agents', { projectId: selectedProject.project_id });

          // 转换为 Member 格式
          const members: Member[] = agents.map(agent => ({
            id: agent.id || '',
            name: agent.nickname || agent.name,
            avatar: agent.icon || undefined,
            role_type: agent.role_type,
          }));
          setProjectMembers(members);

          // 获取成员进程状态
          try {
            const statuses = await api.getProjectMemberStatuses(selectedProject.project_id);
            // 将状态映射到成员
            setProjectMembers(prev => prev.map(member => {
              const statusInfo = statuses.find(s => s.agent_id === member.id);
              return {
                ...member,
                status: statusInfo?.status as "pending" | "running" | "completed" | "stopped" | "error" | undefined,
              };
            }));
          } catch (statusError) {
            console.error('Failed to fetch member statuses:', statusError);
          }
        } catch (error) {
          console.error('Failed to fetch project members:', error);
          setProjectMembers([]);
        }
      } else {
        setProjectMembers([]);
      }
    };

    fetchProjectMembers();
  }, [selectedProject]);

  // 监听成员状态更新事件，实时刷新成员状态
  useEffect(() => {
    if (!selectedProject) return;

    const fetchMemberStatuses = async () => {
      try {
        const statuses = await api.getProjectMemberStatuses(selectedProject.project_id);
        setProjectMembers(prev => prev.map(member => {
          const statusInfo = statuses.find(s => s.agent_id === member.id);
          return {
            ...member,
            status: statusInfo?.status as "pending" | "running" | "completed" | "stopped" | "error" | undefined,
          };
        }));
      } catch (statusError) {
        console.error('Failed to fetch member statuses:', statusError);
      }
    };

    let unlisten: (() => void) | undefined;
    const setupListener = async () => {
      unlisten = await listen(`member-status-update:${selectedProject.project_id}`, () => {
        console.log('[ThreeLevelLayout] Member status updated, refreshing...');
        fetchMemberStatuses();
      });
    };

    setupListener();

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, [selectedProject]);

  // 当选中项目变化时，获取项目消息
  useEffect(() => {
    const fetchMessages = async () => {
      if (selectedProject) {
        try {
          setLoadingMessages(true);
          const msgs = await api.getMessages(selectedProject.project_id);
          setMessages(msgs);
        } catch (error) {
          console.error('Failed to fetch messages:', error);
          setMessages([]);
        } finally {
          setLoadingMessages(false);
        }
      } else {
        setMessages([]);
      }
    };

    fetchMessages();

    // 监听 agent 完成任务的事件，实时刷新消息
    let unlisten: (() => void) | undefined;
    const setupListener = async () => {
      unlisten = await listen<string>('project-message-update', (event) => {
        console.log('[ThreeLevelLayout] Received message update event:', event.payload);
        // 如果事件中的 project_id 与当前选中的项目匹配，则刷新
        if (selectedProject && event.payload === selectedProject.project_id) {
          fetchMessages();
        }
      });
    };

    setupListener();

    // 仅使用 Tauri 事件监听，不再使用定时刷新
    return () => {
      if (unlisten) unlisten();
    };
  }, [selectedProject]);

  // 过滤项目
  const filteredProjects = projects.filter(p =>
    p.project_name.toLowerCase().includes(projectSearchQuery.toLowerCase())
  );

  // 处理新建项目
  const handleCreateProject = (project: { name: string; description: string; workDir: string }) => {
    onAddClick?.(project);
    setShowCreateDialog(false);
  };

  // 拖拽分隔条处理
  const handleDividerMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    setIsDragging(true);
  };

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      if (!isDragging) return;

      // 获取主内容区的容器
      const container = document.getElementById('main-content-container');
      if (!container) return;

      const rect = container.getBoundingClientRect();
      const newPosition = ((e.clientY - rect.top) / rect.height) * 100;

      // 限制范围在 20% - 80% 之间
      if (newPosition >= 20 && newPosition <= 80) {
        setDividerPosition(newPosition);
      }
    };

    const handleMouseUp = () => {
      setIsDragging(false);
    };

    if (isDragging) {
      document.addEventListener('mousemove', handleMouseMove);
      document.addEventListener('mouseup', handleMouseUp);
    }

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [isDragging]);

  // 渲染第一栏 - 全局导航
  const renderGlobalNav = () => (
    <div className="w-14 bg-blue-50/50 flex flex-col items-center py-4 border-r">
      {/* 窗口控制按钮 */}
      {/*<div className="flex gap-1.5 mb-6">*/}
      {/*  <div className="w-3 h-3 rounded-full bg-red-400" />*/}
      {/*  <div className="w-3 h-3 rounded-full bg-yellow-400" />*/}
      {/*  <div className="w-3 h-3 rounded-full bg-green-400" />*/}
      {/*</div>*/}

      {/* 导航图标 */}
      <div className="flex flex-col gap-4 flex-1">
        {globalNavItems.map((item) => (
          <motion.button
            key={item.id}
            whileHover={{ scale: 1.05 }}
            whileTap={{ scale: 0.95 }}
            onClick={() => setSelectedNav(item.id)}
            className={cn(
              "w-10 h-10 rounded-lg flex items-center justify-center transition-colors",
              selectedNav === item.id
                ? "bg-black text-white"
                : "text-gray-500 hover:bg-gray-100"
            )}
            title={item.label}
          >
            {item.icon}
          </motion.button>
        ))}
      </div>
    </div>
  );

  // 渲染第二栏 - 项目列表
  const renderProjectList = () => (
    <div className="w-56 bg-red-50/50 flex flex-col border-r">
      {/* 搜索和添加 */}
      <div className="p-3 border-b">
        <div className="flex gap-2">
          <div className="relative flex-1">
            <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400" />
            <Input
              placeholder="搜索项目"
              value={projectSearchQuery}
              onChange={(e) => setProjectSearchQuery(e.target.value)}
              className="pl-8 h-8 bg-white text-sm"
            />
          </div>
          <motion.button
            whileHover={{ scale: 1.05 }}
            whileTap={{ scale: 0.95 }}
            onClick={() => setShowCreateDialog(true)}
            className="w-8 h-8 bg-red-500 text-white rounded-md flex items-center justify-center"
            title="新建项目"
          >
            <Plus className="w-4 h-4" />
          </motion.button>
        </div>
      </div>

      {/* 项目列表 */}
      <ScrollArea className="flex-1 p-2">
        {filteredProjects.length === 0 ? (
          <div className="text-center py-8 text-gray-400 text-sm">
            <p>暂无项目</p>
            <p className="text-xs mt-1">点击 + 创建新项目</p>
          </div>
        ) : (
          <div className="space-y-2">
            {filteredProjects.map((project) => (
              <motion.div
                key={project.project_id}
                whileHover={project.initializing ? {} : { scale: 1.01 }}
                onClick={() => {
                  if (!project.initializing) {
                    setSelectedProject(project);
                  }
                }}
                className={cn(
                  "p-2 rounded-lg transition-all relative overflow-hidden",
                  project.initializing
                    ? "cursor-not-allowed opacity-50"
                    : "cursor-pointer hover:bg-white/50",
                  selectedProject?.project_id === project.project_id && !project.initializing
                    ? "bg-purple-100/80"
                    : ""
                )}
              >
                {/* 选中高亮背景 */}
                {selectedProject?.project_id === project.project_id && !project.initializing && (
                  <div className="absolute inset-0 bg-purple-500/10" />
                )}

                <div className="flex items-center gap-2 relative">
                  {/* 项目缩略图 */}
                  <div className="w-10 h-10 bg-gradient-to-br from-purple-300 to-blue-300 rounded-md flex items-center justify-center text-white text-xs font-medium">
                    {project.project_name.charAt(0)}
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="font-medium text-sm truncate">{project.project_name}</div>
                    <div className="text-xs text-gray-400 truncate">{project.workspace_path}</div>
                    {/* 进度条 */}
                    {project.initializing && project.progress && (
                      <div className="mt-1">
                        <div className="flex items-center justify-between text-xs text-blue-500 mb-0.5">
                          <span>{project.progress.message}</span>
                        </div>
                        <div className="h-1 bg-gray-200 rounded-full overflow-hidden">
                          <div
                            className="h-full bg-blue-500 transition-all duration-300"
                            style={{
                              width: project.progress.step === 'starting' ? '5%' :
                                     project.progress.step === 'preparing' ? '10%' :
                                     project.progress.step === 'writing_skill' ? '15%' :
                                     project.progress.step === 'finding_claude' ? '20%' :
                                     project.progress.step === 'executing_claude' ? '40%' :
                                     project.progress.step === 'parsing_json' ? '60%' :
                                     project.progress.step === 'saving_agents' ? '80%' :
                                     project.progress.step === 'completed' ? '100%' : '30%'
                            }}
                          />
                        </div>
                      </div>
                    )}
                  </div>
                  {/* Loading 图标 */}
                  {project.initializing && (
                    <Loader2 className="w-4 h-4 animate-spin text-blue-500" />
                  )}
                </div>
              </motion.div>
            ))}
          </div>
        )}
      </ScrollArea>
    </div>
  );

  // 渲染第三栏 - 主内容区
  const renderMainContent = () => (
    <div className="flex-1 bg-white flex flex-col overflow-hidden">
      {selectedProject ? (
        <>
          {/* 顶部标题栏 */}
          <div className="h-14 border-b flex items-center justify-between px-4">
            <div>
              <h1 className="text-lg font-semibold">{selectedProject.project_name}</h1>
              <p className="text-xs text-gray-400">项目描述在这里</p>
            </div>
            <div className="flex items-center gap-2">

              <motion.button
                whileHover={{ scale: 1.05 }}
                whileTap={{ scale: 0.95 }}
                className="w-8 h-8 rounded-md hover:bg-gray-100 flex items-center justify-center gap-1"
              >
                <UserPlus className="w-4 h-4 text-gray-500" />
                {/*<span className="text-xs text-gray-500">邀请</span>*/}
              </motion.button>
              <motion.button
                whileHover={{ scale: 1.05 }}
                whileTap={{ scale: 0.95 }}
                className="w-8 h-8 rounded-md hover:bg-gray-100 flex items-center justify-center"
              >
                <MoreVertical className="w-4 h-4 text-gray-500" />
              </motion.button>
            </div>
          </div>
          <div className="flex-1 flex flex-col overflow-hidden" name={"项目聊天窗口"} id="main-content-container">
            {/* 上部分：中央内容区 - 消息列表 */}
            <div
              className="overflow-hidden flex flex-col"
              style={{ height: `${dividerPosition}%` }}
            >
              {loadingMessages ? (
                <div className="flex-1 flex items-center justify-center">
                  <Loader2 className="w-6 h-6 animate-spin text-blue-500" />
                </div>
              ) : messages.length === 0 ? (
                <div className="flex-1 flex items-center justify-center text-gray-300">
                  <div className="text-center">
                    <MessageSquare className="w-16 h-16 mx-auto mb-4 opacity-50" />
                    <p className="text-lg">暂无消息</p>
                    <p className="text-sm mt-2">发送消息开始对话</p>
                  </div>
                </div>
              ) : (
                <ScrollArea className="flex-1 p-4">
                  <div className="space-y-4">
                    {messages.map((msg) => (
                      <div
                        key={msg.id}
                        className={cn(
                          "flex gap-3",
                          msg.sender_id === 'user' ? "flex-row-reverse" : "flex-row"
                        )}
                      >
                        {/* 头像：显示 icon 图标或名字首字母 */}
                        <div className="flex-shrink-0">
                          <div className={cn(
                            "w-8 h-8 rounded-full flex items-center justify-center text-white",
                            msg.sender_id === 'user'
                              ? "bg-blue-500"
                              : "bg-gradient-to-br from-green-300 to-blue-300"
                          )}>
                            {msg.sender_avatar && ICON_MAP[msg.sender_avatar] ? (
                              React.createElement(ICON_MAP[msg.sender_avatar], { className: "w-5 h-5" })
                            ) : (
                              <span className="text-sm">{msg.sender_name?.charAt(0) || '?'}</span>
                            )}
                          </div>
                        </div>
                        {/* 消息内容 */}
                        <div className={cn(
                          "flex flex-col max-w-[70%]",
                          msg.sender_id === 'user' ? "items-end" : "items-start"
                        )}>
                          <div className="flex items-center gap-2 mb-1">
                            <span className="text-xs font-medium text-gray-600">
                              {msg.sender_id === 'user'
                                ? `You -> ${msg.target_name || 'All'}`
                                : `${msg.sender_name} -> You`}
                            </span>
                            <span className="text-xs text-gray-400">
                              {new Date(msg.created_at).toLocaleTimeString()}
                            </span>
                          </div>
                          <div
                            className={cn(
                              "rounded-lg px-4 py-2 w-full",
                              msg.sender_id === 'user'
                                ? "bg-blue-500 text-white"
                                : msg.message_type === 'thinking'
                                  ? "bg-yellow-100 text-yellow-800"
                                  : "bg-gray-100 text-gray-800"
                            )}
                          >
                            {msg.message_type === 'thinking' && (
                              <div className="text-xs text-yellow-600 mb-1">💭 Thinking</div>
                            )}
                            <p className="text-sm whitespace-pre-wrap whitespace-normal break-words">{msg.content}</p>
                          </div>
                        </div>
                      </div>
                    ))}
                  </div>
                </ScrollArea>
              )}
            </div>

            {/* 拖拽分隔条 */}
            <div
              className={cn(
                "h-1 bg-gray-200 cursor-row-resize hover:bg-blue-400 transition-colors flex-shrink-0",
                isDragging && "bg-blue-500"
              )}
              onMouseDown={handleDividerMouseDown}
            />

            {/* 下部分：输入工具栏 */}
            <div
              className="flex-shrink-0 overflow-visible flex flex-1 flex-col"
              style={{ height: `${100 - dividerPosition}%` }}
            >
              {/* 底部输入工具栏 */}
              <div className="h-12 border-t flex items-center px-4 gap-2">
                <div className="flex items-center gap-1">
                  <motion.button whileHover={{ scale: 1.1 }} className="w-8 h-8 rounded flex items-center justify-center text-gray-400 hover:bg-gray-100">
                    <Smile className="w-4 h-4" />
                  </motion.button>
                  <motion.button whileHover={{ scale: 1.1 }} className="w-8 h-8 rounded flex items-center justify-center text-gray-400 hover:bg-gray-100">
                    <Scissors className="w-4 h-4" />
                  </motion.button>
                  <motion.button whileHover={{ scale: 1.1 }} className="w-8 h-8 rounded flex items-center justify-center text-gray-400 hover:bg-gray-100">
                    <Image className="w-4 h-4" />
                  </motion.button>
                  <motion.button whileHover={{ scale: 1.1 }} className="w-8 h-8 rounded flex items-center justify-center text-gray-400 hover:bg-gray-100">
                    <FileVideo className="w-4 h-4" />
                  </motion.button>
                  <motion.button whileHover={{ scale: 1.1 }} className="w-8 h-8 rounded flex items-center justify-center text-gray-400 hover:bg-gray-100">
                    <ListTodo className="w-4 h-4" />
                  </motion.button>
                  <motion.button whileHover={{ scale: 1.1 }} className="w-8 h-8 rounded flex items-center justify-center text-gray-400 hover:bg-gray-100">
                    <FolderPlus className="w-4 h-4" />
                  </motion.button>
                  <motion.button whileHover={{ scale: 1.1 }} className="w-8 h-8 rounded flex items-center justify-center text-gray-400 hover:bg-gray-100">
                    <MoreHorizontal className="w-4 h-4" />
                  </motion.button>
                </div>

                <div className="flex-1" />

                <div className="flex items-center gap-2">
                  <motion.button
                    whileHover={{ scale: 1.02 }}
                    whileTap={{ scale: 0.98 }}
                    className="h-8 px-3 bg-blue-500 text-white rounded-md flex items-center gap-1.5 text-sm"
                  >
                    <Zap className="w-3.5 h-3.5" />
                    快速会议
                  </motion.button>
                  <motion.button whileHover={{ scale: 1.1 }} className="w-8 h-8 rounded flex items-center justify-center text-gray-400 hover:bg-gray-100">
                    <BookOpen className="w-4 h-4" />
                  </motion.button>
                </div>
              </div>
              <div className="w-full flex-1 px-0 bg-red">
                <FloatingPromptInput
                  onSend={handleSendMessage}
                  projectId={selectedProject?.project_id}
                  projectPath={selectedProject?.workspace_path}
                  isLoading={isSending}
                  disabled={!selectedProject}
                />
              </div>
            </div>
          </div>
        </>
      ) : (
        <div className="flex-1 flex items-center justify-center text-gray-300">
          <div className="text-center">
            <FolderOpen className="w-16 h-16 mx-auto mb-4 opacity-50" />
            <p className="text-lg">选择一个项目开始</p>
          </div>
        </div>
      )}
    </div>
  );

  // 渲染第四栏 - 成员列表
  const renderMemberList = () => (
    <div className="w-56 bg-green-50/50 flex flex-col border-l">
      {/* 顶部标题 */}
      <div className="h-14 border-b flex items-center justify-between px-4">
        <span className="text-sm font-medium">群成员 · {projectMembers.length}</span>
        <div className="flex items-center gap-1">
          <motion.button whileHover={{ scale: 1.1 }} className="w-7 h-7 rounded flex items-center justify-center text-gray-400 hover:bg-gray-100">
            <Search className="w-3.5 h-3.5" />
          </motion.button>
          <motion.button whileHover={{ scale: 1.1 }}
                         title="添加成员"
                         onClick={handleAddMemberClick}
                         className="w-7 h-7 rounded flex items-center justify-center text-gray-400 hover:bg-gray-100">
            <Plus className="w-3.5 h-3.5" />
          </motion.button>
        </div>
      </div>

      {/* 成员列表 - teamlead 排在第一位 */}
      <ScrollArea className="flex-1 p-3">
        {projectMembers.length > 0 ? (
          <div className="space-y-2">
            {([...projectMembers].sort((a, b) => {
              if (a.role_type === 'teamlead' && b.role_type !== 'teamlead') return -1;
              if (a.role_type !== 'teamlead' && b.role_type === 'teamlead') return 1;
              return 0;
            })).map((member) => (
              <div key={member.id} className="flex items-center gap-3 p-2 rounded-lg hover:bg-white/50 cursor-pointer">
                {/* 头像：显示 icon 图标或名字首字母 */}
                <div className="w-8 h-8 bg-gradient-to-br from-green-300 to-blue-300 rounded-full flex items-center justify-center text-white">
                  {member.avatar && ICON_MAP[member.avatar] ? (
                    React.createElement(ICON_MAP[member.avatar], { className: "w-5 h-5" })
                  ) : (
                    <span className="text-sm">{member.name.charAt(0)}</span>
                  )}
                </div>
                <span className="text-sm flex-1">{member.name}</span>
                {/* 状态指示点 */}
                <div
                  className={cn(
                    "w-2 h-2 rounded-full flex-shrink-0",
                    member.status === 'running' ? "bg-green-500" :
                    member.status === 'pending' ? "bg-yellow-500" :
                    member.status === 'completed' ? "bg-blue-500" :
                    member.status === 'error' ? "bg-red-500" :
                    "bg-gray-400"
                  )}
                  title={
                    member.status === 'running' ? "运行中" :
                    member.status === 'pending' ? "等待中" :
                    member.status === 'completed' ? "已完成" :
                    member.status === 'error' ? "错误" :
                    "未启动"
                  }
                />
                {member.role_type === 'teamlead' && (
                  <span className="text-xs bg-amber-100 text-amber-700 px-1.5 py-0.5 rounded">Lead</span>
                )}
              </div>
            ))}
          </div>
        ) : (
          <div className="text-center py-8 text-gray-400">
            <Users className="w-12 h-12 mx-auto mb-2 opacity-50" />
            <p className="text-sm">项目成员列表</p>
          </div>
        )}
      </ScrollArea>
    </div>
  );

  // 添加成员 Modal
  const renderAddMemberModal = () => (
    <Dialog open={showAddMemberModal} onOpenChange={setShowAddMemberModal}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>添加项目成员</DialogTitle>
          <DialogDescription>
            选择要添加到项目的成员（不包括 Team Lead）
          </DialogDescription>
        </DialogHeader>
        <div className="max-h-[300px] overflow-y-auto py-2">
          {loadingAvailableMembers ? (
            <div className="flex items-center justify-center py-8">
              <Loader2 className="w-6 h-6 animate-spin text-muted-foreground" />
            </div>
          ) : availableMembers.length === 0 ? (
            <div className="text-center py-8 text-muted-foreground">
              <Users className="w-12 h-12 mx-auto mb-2 opacity-50" />
              <p>暂无可添加的成员</p>
              <p className="text-xs mt-1">所有成员已加入或无可用成员</p>
            </div>
          ) : (
            <div className="space-y-2">
              {availableMembers.map((agent) => {
                const agentId = agent.id || '';
                const isSelected = !!agentId && selectedMemberIds.includes(agentId);
                const handleClick = () => {
                  console.log('[AddMember] Clicked agent:', agent.name, 'id:', agent.id, 'agentId:', agentId);
                  if (agentId) {
                    toggleMemberSelection(agentId);
                  }
                };
                return (
                  <div
                    key={agent.id}
                    onClick={handleClick}
                    className={cn(
                      "flex items-center gap-3 p-3 rounded-lg cursor-pointer transition-colors",
                      isSelected ? "bg-primary/10 border border-primary" : "hover:bg-accent"
                    )}
                  >
                    <div className={cn(
                      "w-5 h-5 rounded border-2 flex items-center justify-center transition-colors",
                      isSelected ? "bg-primary border-primary" : "border-muted-foreground"
                    )}>
                      {isSelected && <Check className="w-3 h-3 text-white" />}
                    </div>
                    <div className="w-10 h-10 bg-gradient-to-br from-green-300 to-blue-300 rounded-full flex items-center justify-center text-white text-sm font-medium">
                      {agent.name.charAt(0)}
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="font-medium text-sm truncate">{agent.name}</div>
                      <div className="text-xs text-muted-foreground">
                        {agent.role_type || 'general'}
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => setShowAddMemberModal(false)}>
            取消
          </Button>
          <Button
            onClick={handleAddSelectedMembers}
            disabled={selectedMemberIds.length === 0 || isAddingMembers}
          >
            {isAddingMembers ? (
              <Loader2 className="w-4 h-4 animate-spin mr-2" />
            ) : null}
            添加 {selectedMemberIds.length > 0 ? `(${selectedMemberIds.length})` : ''}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );

  // 渲染设置页面 - 使用 SettingsComponent 内部的 Tabs 作为二级导航
  const renderSettingsPage = () => (
    <div className="flex-1 bg-white overflow-hidden">
      <SettingsComponent
        onBack={() => setSelectedNav('projects')}
      />
    </div>
  );

  // 渲染成员管理页面 - 使用 Teammates 组件
  const renderTeamPage = () => (
    <div className="flex-1 bg-white overflow-hidden">
      <Teammates
        onBack={() => setSelectedNav('projects')}
        className="h-full"
      />
    </div>
  );

  // 渲染项目页面 - 二级布局
  const renderProjectPage = () => (
    <>
      {/* 项目页面 - 二级导航（项目列表） */}
      {renderProjectList()}

      {/* 项目页面 - 二级主体 */}
      {renderMainContent()}

      {/* 项目页面 - 第四栏（成员列表） */}
      {selectedProject && renderMemberList()}
    </>
  );

  return (
    <>
      <CreateProjectDialog
        isOpen={showCreateDialog}
        onClose={() => setShowCreateDialog(false)}
        onConfirm={handleCreateProject}
        isLoading={isCreatingProject}
        onLoadingChange={onCreatingProjectChange}
      />
      {renderAddMemberModal()}
      <div className={cn("flex h-full", className)}>
        {/* 第一栏：全局导航 */}
        {renderGlobalNav()}

        {/* 主体部分：根据选中的全局导航项显示对应页面 */}
        {selectedNav === 'settings' ? (
          renderSettingsPage()
        ) : selectedNav === 'team' ? (
          renderTeamPage()
        ) : (
          renderProjectPage()
        )}
      </div>
    </>
  );
};
