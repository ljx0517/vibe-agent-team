import React, { useEffect, useState } from 'react';
import { motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import {
  FolderOpen, FileText, Users, BarChart, MessageSquare, Settings,
  Search, Plus, MoreVertical, UserPlus, Smile, Scissors,
  Image, FileVideo, ListTodo, FolderPlus, MoreHorizontal,
  Zap, BookOpen, Loader2, Bot
} from 'lucide-react';
import { cn } from '@/lib/utils';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Input } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import { CreateProjectDialog } from '@/components/CreateProjectDialog';
import { Settings as SettingsComponent } from '@/components/Settings';
import { FloatingPromptInput } from './FloatingPromptInput';
import { api, type Agent } from '@/lib/api';
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
  const [agents, setAgents] = useState<Agent[]>([]);
  const [agentsLoading, setAgentsLoading] = useState(false);

  // 加载系统 agents
  const loadAgents = async () => {
    try {
      setAgentsLoading(true);
      const agentsList = await api.listAgents();
      setAgents(agentsList);
    } catch (error) {
      console.error('Failed to load agents:', error);
    } finally {
      setAgentsLoading(false);
    }
  };

  // 当选中 team 导航时加载 agents
  useEffect(() => {
    if (selectedNav === 'team') {
      loadAgents();
    }
  }, [selectedNav]);

  // 发送消息处理函数
  const handleSendMessage = async (text: string, _model: "sonnet" | "opus") => {
    if (!selectedProject || !text.trim()) return;

    setIsSending(true);
    try {
      await api.sendMessage(selectedProject.project_id, text.trim());
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
          }>>('list_project_agents', { projectId: selectedProject.project_id });

          // 转换为 Member 格式
          const members: Member[] = agents.map(agent => ({
            id: agent.id || '',
            name: agent.nickname || agent.name,
            avatar: agent.icon || undefined,
          }));
          setProjectMembers(members);
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
    <div className="flex-1 bg-white flex flex-col">
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
          <div className="flex-1 flex flex-col overflow-hidden" id="main-content-container">
            {/* 上部分：中央内容区 */}
            <div
              className="overflow-hidden"
              style={{ height: `${dividerPosition}%` }}
            >
              <div className="h-full w-full flex items-center justify-center text-gray-300">
                <div className="text-center">
                  <MessageSquare className="w-16 h-16 mx-auto mb-4 opacity-50" />
                  <p className="text-lg">中央内容区</p>
                </div>
              </div>
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
                         className="w-7 h-7 rounded flex items-center justify-center text-gray-400 hover:bg-gray-100">
            <Plus className="w-3.5 h-3.5" />
          </motion.button>
        </div>
      </div>

      {/* 成员列表 */}
      <ScrollArea className="flex-1 p-3">
        {projectMembers.length > 0 ? (
          <div className="space-y-2">
            {projectMembers.map((member) => (
              <div key={member.id} className="flex items-center gap-3 p-2 rounded-lg hover:bg-white/50 cursor-pointer">
                <div className="w-8 h-8 bg-gradient-to-br from-green-300 to-blue-300 rounded-full flex items-center justify-center text-white text-xs">
                  {member.name.charAt(0)}
                </div>
                <span className="text-sm">{member.name}</span>
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

  // 渲染设置页面 - 使用 SettingsComponent 内部的 Tabs 作为二级导航
  const renderSettingsPage = () => (
    <div className="flex-1 bg-white overflow-hidden">
      <SettingsComponent
        onBack={() => setSelectedNav('projects')}
      />
    </div>
  );

  // 渲染成员管理页面 - 显示系统内所有 agents
  const renderTeamPage = () => (
    <div className="flex-1 bg-white overflow-hidden flex flex-col">
      {/* 页面头部 */}
      <div className="p-6 border-b">
        <div className="flex items-center gap-3">
          <Button
            variant="ghost"
            size="icon"
            onClick={() => setSelectedNav('projects')}
            className="h-8 w-8"
          >
            <FolderOpen className="h-4 w-4" />
          </Button>
          <div>
            <h1 className="text-2xl font-bold">成员管理</h1>
            <p className="text-sm text-muted-foreground">
              系统内所有 Agent 成员
            </p>
          </div>
        </div>
      </div>

      {/* Agent 列表 */}
      <ScrollArea className="flex-1">
        <div className="p-6">
          {agentsLoading ? (
            <div className="flex items-center justify-center h-64">
              <Loader2 className="w-8 h-8 animate-spin text-muted-foreground" />
            </div>
          ) : agents.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-64 text-center">
              <Bot className="w-12 h-12 text-muted-foreground mb-4" />
              <h3 className="text-lg font-semibold mb-2">暂无 Agent 成员</h3>
              <p className="text-sm text-muted-foreground">
                创建您的第一个 Agent 成员
              </p>
            </div>
          ) : (
            <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
              {agents.map((agent) => (
                <div
                  key={agent.id}
                  className="p-4 border rounded-lg hover:shadow-md transition-shadow"
                >
                  <div className="flex items-start justify-between mb-3">
                    <div className="flex items-center gap-2">
                      <Bot className="w-5 h-5 text-primary" />
                      <h3 className="font-semibold">{agent.name}</h3>
                    </div>
                  </div>
                  <p className="text-sm text-muted-foreground line-clamp-2 mb-3">
                    {agent.system_prompt?.slice(0, 100) || '暂无描述'}
                  </p>
                  <div className="flex items-center justify-between">
                    <span className="text-xs text-muted-foreground">
                      创建于 {new Date(agent.created_at).toLocaleDateString()}
                    </span>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </ScrollArea>
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
