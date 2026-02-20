import React, { useState } from "react";
import { Folder } from "lucide-react";
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

interface CreateProjectDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onConfirm: (project: { name: string; description: string; workDir: string }) => void;
}

export const CreateProjectDialog: React.FC<CreateProjectDialogProps> = ({
  isOpen,
  onClose,
  onConfirm,
}) => {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [workDir, setWorkDir] = useState("");
  const [nameError, setNameError] = useState(false);

  const handleOpenFolder = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        directory: true,
        multiple: false,
        title: "选择工作目录",
      });
      if (selected) {
        setWorkDir(selected as string);
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
    onConfirm({
      name: name.trim(),
      description: description.trim(),
      workDir: workDir.trim(),
    });
    // 重置表单
    setName("");
    setDescription("");
    setWorkDir("");
    setNameError(false);
  };

  const handleClose = () => {
    setName("");
    setDescription("");
    setWorkDir("");
    setNameError(false);
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

          {/* 工作目录 */}
          <div className="grid gap-2">
            <Label>工作目录</Label>
            <div className="flex gap-2">
              <Input
                placeholder="点击右侧按钮选择文件夹"
                value={workDir}
                readOnly
                className="flex-1 bg-muted"
              />
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
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={handleClose}>
            取消
          </Button>
          <Button onClick={handleConfirm}>创建</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
