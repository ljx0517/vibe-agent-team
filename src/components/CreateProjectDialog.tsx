import React, { useState, useEffect } from "react";
import { Folder, AlertCircle, CheckCircle } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

// Extended agent type with additional fields from backend
interface TeamleadAgent {
  id?: number | string;
  project_id?: string;
  name: string;
  icon?: string;
  color?: string;
  nickname?: string;
  gender?: string;
  agent_type?: string;
  description?: string;
  system_prompt?: string;
  default_task?: string;
  model?: string;
  tools?: string;
  enable_file_read?: boolean;
  enable_file_write?: boolean;
  enable_network?: boolean;
  hooks?: string;
  settings?: string;
  created_at?: string;
  updated_at?: string;
}

interface DirectoryStatus {
  is_empty: boolean;
  has_workspace_marker: boolean;
  is_valid_workspace: boolean;
  error: string | null;
}

interface SelectedTeamlead {
  id: string;
  name: string;
  nickname?: string;
  gender?: string;
  prompt?: string;
  model?: string;
  color?: string;
}

interface CreateProjectDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onConfirm: (project: { name: string; projectCode?: string; description: string; workDir: string; teamlead?: SelectedTeamlead }) => void;
  isLoading?: boolean;
  onLoadingChange?: (isLoading: boolean) => void;
}

export const CreateProjectDialog: React.FC<CreateProjectDialogProps> = ({
  isOpen,
  onClose,
  onConfirm,
  isLoading = false,
  onLoadingChange,
}) => {
  const [name, setName] = useState("");
  const [projectCode, setProjectCode] = useState("");
  const [description, setDescription] = useState("");
  const [workDir, setWorkDir] = useState("");
  const [nameError, setNameError] = useState(false);
  const [workDirError, setWorkDirError] = useState<string | null>(null);
  const [workDirStatus, setWorkDirStatus] = useState<"empty" | "workspace" | "invalid" | null>(null);
  const [teamleads, setTeamleads] = useState<TeamleadAgent[]>([]);
  const [selectedTeamlead, setSelectedTeamlead] = useState<string>("new");

  // Load teamleads when dialog opens
  useEffect(() => {
    if (isOpen) {
      api.listTeamleads()
        .then((data) => setTeamleads(data as unknown as TeamleadAgent[]))
        .catch(err => console.error("Failed to load teamleads:", err));
    }
  }, [isOpen]);

  const handleOpenFolder = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        directory: true,
        multiple: false,
        title: "选择工作空间",
      });
      if (selected) {
        const path = selected as string;
        setWorkDir(path);
        setWorkDirError(null);
        setWorkDirStatus(null);

        // Check directory status
        const status = await invoke<DirectoryStatus>("check_directory_status", { path });

        if (status.error) {
          setWorkDirError(status.error);
          setWorkDirStatus("invalid");
          return;
        }

        if (status.has_workspace_marker) {
          // Existing workspace
          setWorkDirStatus("workspace");
          setWorkDirError(null);
        } else if (status.is_empty) {
          // Empty directory - create workspace marker
          await invoke("create_workspace_marker", { path });
          setWorkDirStatus("empty");
          setWorkDirError(null);
        } else {
          // Not empty and no workspace marker
          setWorkDirError("请选择空文件夹或已初始化的工作空间");
          setWorkDirStatus("invalid");
        }
      }
    } catch (error) {
      console.error("Failed to open folder dialog:", error);
    }
  };

  const handleConfirm = () => {
    if (!name.trim()) {
      setNameError(true);
      return;
    }
    if (!workDir.trim()) {
      setWorkDirError("请选择工作空间");
      return;
    }
    if (workDirStatus === "invalid") {
      setWorkDirError("请选择空文件夹或已初始化的工作空间");
      return;
    }
    // 设置 loading 状态
    onLoadingChange?.(true);

    // 处理 teamlead 选择
    let teamlead: SelectedTeamlead | undefined;
    if (selectedTeamlead !== "new") {
      const selected = teamleads.find(t => String(t.id) === selectedTeamlead);
      if (selected) {
        teamlead = {
          id: String(selected.id || ""),
          name: selected.name,
          nickname: selected.nickname,
          gender: selected.gender,
          prompt: selected.system_prompt || "",
          model: selected.model,
          color: selected.color,
        };
      }
    }

    onConfirm({
      name: name.trim(),
      projectCode: projectCode.trim() || undefined,
      description: description.trim(),
      workDir: workDir.trim(),
      teamlead,
    });
    // 重置表单（保持 loading 状态由父组件控制）
    setName("");
    setProjectCode("");
    setDescription("");
    setWorkDir("");
    setNameError(false);
    setWorkDirError(null);
    setWorkDirStatus(null);
    setSelectedTeamlead("new");
  };

  const handleClose = () => {
    setName("");
    setProjectCode("");
    setDescription("");
    setWorkDir("");
    setNameError(false);
    setWorkDirError(null);
    setWorkDirStatus(null);
    onClose();
  };

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && handleClose()}>
      <DialogContent className="sm:max-w-[500px]">
        <DialogHeader>
          <DialogTitle>新建项目</DialogTitle>
          <DialogDescription>
            创建一个新的项目，填写以下信息。
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-4 py-4">
          {/* 项目名 */}
          <div className="grid gap-2">
            <Label htmlFor="project-name">
              项目名 <span className="text-red-500">*</span>
            </Label>
            <Input
              id="project-name"
              placeholder="请输入项目名称"
              value={name}
              onChange={(e) => {
                setName(e.target.value);
                if (e.target.value.trim()) {
                  setNameError(false);
                }
              }}
              className={nameError ? "border-red-500" : ""}
            />
            {nameError && (
              <p className="text-xs text-red-500">请输入项目名称</p>
            )}
          </div>

          {/* 项目代码 - 暂时隐藏 */}
          {/* <div className="grid gap-2">
            <Label htmlFor="project-code">项目代码</Label>
            <Input
              id="project-code"
              placeholder="请输入项目代码（选填）"
              value={projectCode}
              onChange={(e) => setProjectCode(e.target.value)}
            />
          </div> */}

          {/* Team Lead 选择 */}
          <div className="grid gap-2">
            <Label htmlFor="teamlead">Team Lead</Label>
            <Select value={selectedTeamlead} onValueChange={setSelectedTeamlead}>
              <SelectTrigger>
                <SelectValue placeholder="选择 Team Lead" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="new">New Teamlead（新建）</SelectItem>
                {teamleads.map((teamlead) => (
                  <SelectItem key={String(teamlead.id)} value={String(teamlead.id || "")}>
                    {teamlead.nickname || teamlead.name} ({teamlead.name})
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {/* 项目描述 */}
          <div className="grid gap-2">
            <Label htmlFor="project-description">项目描述</Label>
            <Textarea
              id="project-description"
              placeholder="请输入项目描述（选填）"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={3}
            />
          </div>

          {/* 工作空间 */}
          <div className="grid gap-2">
            <Label>工作空间</Label>
            <div className="flex gap-2">
              <div className="relative flex-1">
                <Input
                  placeholder="点击右侧按钮选择文件夹"
                  value={workDir}
                  readOnly
                  className={`flex-1 bg-muted pr-10 ${
                    workDirStatus === "workspace"
                      ? "border-green-500"
                      : workDirStatus === "invalid"
                      ? "border-red-500"
                      : ""
                  }`}
                />
                {workDir && (
                  <div className="absolute right-3 top-1/2 -translate-y-1/2">
                    {workDirStatus === "workspace" ? (
                      <CheckCircle className="w-4 h-4 text-green-500" />
                    ) : workDirStatus === "empty" ? (
                      <CheckCircle className="w-4 h-4 text-blue-500" />
                    ) : workDirStatus === "invalid" ? (
                      <AlertCircle className="w-4 h-4 text-red-500" />
                    ) : null}
                  </div>
                )}
              </div>
              <Button
                type="button"
                variant="outline"
                onClick={handleOpenFolder}
                className="gap-2"
              >
                <Folder className="w-4 h-4" />
                选择
              </Button>
            </div>
            {workDirError && (
              <p className="text-xs text-red-500">{workDirError}</p>
            )}
            {workDirStatus === "workspace" && (
              <p className="text-xs text-green-600">已识别为工作空间</p>
            )}
            {workDirStatus === "empty" && (
              <p className="text-xs text-blue-600">已创建新的工作空间</p>
            )}
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={handleClose} disabled={isLoading}>
            取消
          </Button>
          <Button onClick={handleConfirm} disabled={isLoading}>
            {isLoading ? "创建中..." : "创建"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
