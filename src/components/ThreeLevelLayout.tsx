import React, { useState } from 'react';
import { motion } from 'framer-motion';
import { ChevronRight, Search, FolderOpen, FileText, Settings, Users, BarChart, MessageSquare, Plus } from 'lucide-react';
import { cn } from '@/lib/utils';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Input } from '@/components/ui/input';

// 一级菜单项类型
interface Level1Item {
  id: string;
  label: string;
  icon: React.ReactNode;
  children: Level2Item[];
}

// 二级菜单项类型
interface Level2Item {
  id: string;
  label: string;
  children: Level3Item[];
}

// 三级菜单项类型
interface Level3Item {
  id: string;
  label: string;
  content?: React.ReactNode;
}

// 预定义的菜单数据
const defaultMenuData: Level1Item[] = [
  {
    id: 'projects',
    label: '项目',
    icon: <FolderOpen className="w-5 h-5" />,
    children: [
      {
        id: 'project-list',
        label: '项目列表',
        children: [
          { id: 'all-projects', label: '所有项目' },
          { id: 'recent-projects', label: '最近项目' },
          { id: 'starred-projects', label: '星标项目' },
        ],
      },
      {
        id: 'project-settings',
        label: '项目设置',
        children: [
          { id: 'general', label: '常规设置' },
          { id: 'git-config', label: 'Git 配置' },
          { id: 'env-vars', label: '环境变量' },
        ],
      },
    ],
  },
  {
    id: 'documents',
    label: '文档',
    icon: <FileText className="w-5 h-5" />,
    children: [
      {
        id: 'notes',
        label: '笔记',
        children: [
          { id: 'all-notes', label: '所有笔记' },
          { id: 'notebooks', label: '笔记本' },
          { id: 'tags', label: '标签' },
        ],
      },
      {
        id: 'claude-files',
        label: 'Claude 文件',
        children: [
          { id: 'claude-md', label: 'CLAUDE.md' },
          { id: 'instructions', label: '自定义指令' },
        ],
      },
    ],
  },
  {
    id: 'team',
    label: '团队',
    icon: <Users className="w-5 h-5" />,
    children: [
      {
        id: 'members',
        label: '成员管理',
        children: [
          { id: 'member-list', label: '成员列表' },
          { id: 'roles', label: '角色权限' },
          { id: 'invite', label: '邀请成员' },
        ],
      },
      {
        id: 'collaboration',
        label: '协作',
        children: [
          { id: 'shared-projects', label: '共享项目' },
          { id: 'activity', label: '活动记录' },
        ],
      },
    ],
  },
  {
    id: 'analytics',
    label: '分析',
    icon: <BarChart className="w-5 h-5" />,
    children: [
      {
        id: 'usage',
        label: '使用统计',
        children: [
          { id: 'token-usage', label: 'Token 使用量' },
          { id: 'cost-analysis', label: '成本分析' },
          { id: 'sessions', label: '会话统计' },
        ],
      },
    ],
  },
  {
    id: 'messages',
    label: '消息',
    icon: <MessageSquare className="w-5 h-5" />,
    children: [
      {
        id: 'inbox',
        label: '收件箱',
        children: [
          { id: 'all-messages', label: '所有消息' },
          { id: 'unread', label: '未读消息' },
          { id: 'archived', label: '已归档' },
        ],
      },
    ],
  },
  {
    id: 'settings',
    label: '设置',
    icon: <Settings className="w-5 h-5" />,
    children: [
      {
        id: 'general-settings',
        label: '通用设置',
        children: [
          { id: 'appearance', label: '外观' },
          { id: 'shortcuts', label: '快捷键' },
          { id: 'language', label: '语言' },
        ],
      },
      {
        id: 'account',
        label: '账户',
        children: [
          { id: 'profile', label: '个人资料' },
          { id: 'billing', label: '账单' },
          { id: 'security', label: '安全' },
        ],
      },
    ],
  },
];

interface ThreeLevelLayoutProps {
  menuData?: Level1Item[];
  className?: string;
  onMenuChange?: (level1: Level1Item | null, level2: Level2Item | null, level3: Level3Item | null) => void;
  customLevel3Content?: (level3: Level3Item) => React.ReactNode;
  onAddClick?: () => void;
  addButtonText?: string;
}

export const ThreeLevelLayout: React.FC<ThreeLevelLayoutProps> = ({
  menuData = defaultMenuData,
  className,
  onMenuChange,
  customLevel3Content,
  onAddClick,
  addButtonText = '新建',
}) => {
  const [selectedLevel1, setSelectedLevel1] = useState<Level1Item | null>(menuData[0] || null);
  const [selectedLevel2, setSelectedLevel2] = useState<Level2Item | null>(menuData[0]?.children[0] || null);
  const [selectedLevel3, setSelectedLevel3] = useState<Level3Item | null>(
    menuData[0]?.children[0]?.children[0] || null
  );
  const [level2SearchQuery, setLevel2SearchQuery] = useState('');

  // 处理一级菜单点击
  const handleLevel1Click = (item: Level1Item) => {
    setSelectedLevel1(item);
    // 默认选择第一个二级菜单
    const firstLevel2 = item.children[0];
    if (firstLevel2) {
      setSelectedLevel2(firstLevel2);
      // 默认选择第一个三级菜单
      const firstLevel3 = firstLevel2.children[0];
      setSelectedLevel3(firstLevel3 || null);
    } else {
      setSelectedLevel2(null);
      setSelectedLevel3(null);
    }
    onMenuChange?.(item, null, null);
  };

  // 处理二级菜单点击
  const handleLevel2Click = (item: Level2Item) => {
    setSelectedLevel2(item);
    // 默认选择第一个三级菜单
    const firstLevel3 = item.children[0];
    setSelectedLevel3(firstLevel3 || null);
    onMenuChange?.(selectedLevel1, item, null);
  };

  // 处理三级菜单点击
  const handleLevel3Click = (item: Level3Item) => {
    setSelectedLevel3(item);
    onMenuChange?.(selectedLevel1, selectedLevel2, item);
  };

  // 过滤二级菜单
  const filteredLevel2Items = selectedLevel1?.children.filter((item) =>
    item.label.toLowerCase().includes(level2SearchQuery.toLowerCase())
  ) || [];

  // 生成默认的三级内容
  const renderDefaultLevel3Content = (item: Level3Item) => (
    <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
      <div className="text-center">
        <h2 className="text-2xl font-semibold mb-2">{item.label}</h2>
        <p className="text-sm">
          {selectedLevel1?.label} / {selectedLevel2?.label} / {item.label}
        </p>
        <p className="mt-4 text-sm">这是 {item.label} 的内容区域</p>
        <p className="mt-2 text-xs">在此处添加您的自定义内容</p>
      </div>
    </div>
  );

  return (
    <div className={cn("flex h-full", className)}>
      {/* 第一级：主菜单 */}
      <div className="w-56 flex-shrink-0 border-r bg-muted/10">
        <ScrollArea className="h-full">
          <div className="p-2">
            {menuData.map((item) => (
              <motion.button
                key={item.id}
                onClick={() => handleLevel1Click(item)}
                className={cn(
                  "w-full flex items-center gap-3 px-3 py-2.5 rounded-md text-sm font-medium transition-colors",
                  selectedLevel1?.id === item.id
                    ? "bg-primary text-primary-foreground"
                    : "text-muted-foreground hover:bg-muted hover:text-foreground"
                )}
                whileTap={{ scale: 0.98 }}
              >
                {item.icon}
                <span>{item.label}</span>
              </motion.button>
            ))}
          </div>
        </ScrollArea>
      </div>

      {/* 第二级：子菜单区域 */}
      <div className="w-64 flex-shrink-0 border-r bg-muted/5">
        <div className="p-3 border-b">
          <div className="flex items-center gap-2">
            <div className="relative flex-1">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" />
              <Input
                placeholder="搜索..."
                value={level2SearchQuery}
                onChange={(e) => setLevel2SearchQuery(e.target.value)}
                className="pl-9 h-9"
              />
            </div>
            {onAddClick && (
              <motion.button
                onClick={onAddClick}
                whileTap={{ scale: 0.95 }}
                className="h-9 px-3 flex items-center gap-1.5 bg-primary text-primary-foreground rounded-md hover:bg-primary/90 transition-colors text-sm font-medium"
                title={addButtonText}
              >
                <Plus className="w-4 h-4" />
                <span className="hidden sm:inline">{addButtonText}</span>
              </motion.button>
            )}
          </div>
        </div>
        <ScrollArea className="h-[calc(100%-57px)]">
          <div className="p-2">
            {filteredLevel2Items.map((item) => (
              <div key={item.id}>
                <motion.button
                  onClick={() => handleLevel2Click(item)}
                  className={cn(
                    "w-full flex items-center justify-between px-3 py-2 rounded-md text-sm font-medium transition-colors",
                    selectedLevel2?.id === item.id
                      ? "bg-accent text-accent-foreground"
                      : "text-muted-foreground hover:bg-muted hover:text-foreground"
                  )}
                  whileTap={{ scale: 0.98 }}
                >
                  <span>{item.label}</span>
                  <ChevronRight className="w-4 h-4" />
                </motion.button>

                {/* 第三级菜单（作为二级菜单的展开项） */}
                {selectedLevel2?.id === item.id && (
                  <motion.div
                    initial={{ opacity: 0, height: 0 }}
                    animate={{ opacity: 1, height: 'auto' }}
                    exit={{ opacity: 0, height: 0 }}
                    transition={{ duration: 0.2 }}
                    className="ml-2 mt-1 space-y-1 border-l-2 border-muted pl-2"
                  >
                    {item.children.map((level3Item) => (
                      <motion.button
                        key={level3Item.id}
                        onClick={() => handleLevel3Click(level3Item)}
                        className={cn(
                          "w-full text-left px-3 py-1.5 rounded-md text-sm transition-colors",
                          selectedLevel3?.id === level3Item.id
                            ? "bg-primary/10 text-primary font-medium"
                            : "text-muted-foreground hover:bg-muted hover:text-foreground"
                        )}
                        whileTap={{ scale: 0.98 }}
                      >
                        {level3Item.label}
                      </motion.button>
                    ))}
                  </motion.div>
                )}
              </div>
            ))}
          </div>
        </ScrollArea>
      </div>

      {/* 第三级：主内容区域 */}
      <div className="flex-1 overflow-hidden bg-background">
        <ScrollArea className="h-full">
          <div className="h-full p-6">
            {selectedLevel3 ? (
              customLevel3Content ? (
                customLevel3Content(selectedLevel3)
              ) : (
                renderDefaultLevel3Content(selectedLevel3)
              )
            ) : (
              <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
                <div className="text-center">
                  <h2 className="text-xl font-semibold">选择菜单</h2>
                  <p className="text-sm mt-2">请从左侧选择菜单项</p>
                </div>
              </div>
            )}
          </div>
        </ScrollArea>
      </div>
    </div>
  );
};

export default ThreeLevelLayout;
