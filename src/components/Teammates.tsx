import React, { useState, useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  Users,
  Plus,
  Edit,
  Trash2,
  ArrowLeft,
  Loader2,
  ChevronDown,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardFooter } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { api, type Agent } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Toast, ToastContainer } from "@/components/ui/toast";
import { CreateAgent } from "./CreateAgent";
import { ICON_MAP } from "./IconPicker";

interface TeammatesProps {
  /**
   * Callback to go back to the main view
   */
  onBack: () => void;
  /**
   * Optional className for styling
   */
  className?: string;
}

// Available icons for teammates - using all icons from IconPicker
export const TEAMMATE_ICONS = ICON_MAP;

export type TeammateIconName = keyof typeof TEAMMATE_ICONS;

type ViewMode = "list" | "create" | "edit";

/**
 * Teammates component for managing team member agents
 *
 * @example
 * <Teammates onBack={() => setView('home')} />
 */
export const Teammates: React.FC<TeammatesProps> = ({ onBack, className }) => {
  const [teammates, setTeammates] = useState<Agent[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<{ message: string; type: "success" | "error" } | null>(null);
  const [view, setView] = useState<ViewMode>("list");
  const [selectedTeammate, setSelectedTeammate] = useState<Agent | null>(null);
  const [showDeleteDialog, setShowDeleteDialog] = useState(false);
  const [teammateToDelete, setTeammateToDelete] = useState<Agent | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  useEffect(() => {
    loadTeammates();
  }, []);

  const loadTeammates = async () => {
    try {
      setLoading(true);
      setError(null);
      // Load all agents (teamlead, teammate, general, etc.)
      const teammatesList = await api.listAgents();
      setTeammates(teammatesList);
    } catch (err) {
      console.error("Failed to load teammates:", err);
      setError("Failed to load members");
      setToast({ message: "Failed to load members", type: "error" });
    } finally {
      setLoading(false);
    }
  };

  /**
   * Initiates the delete teammate process by showing the confirmation dialog
   */
  const handleDeleteTeammate = (teammate: Agent) => {
    setTeammateToDelete(teammate);
    setShowDeleteDialog(true);
  };

  /**
   * Confirms and executes the teammate deletion
   */
  const confirmDeleteTeammate = async () => {
    if (!teammateToDelete?.id) return;

    try {
      setIsDeleting(true);
      await api.deleteAgent(teammateToDelete.id);
      setToast({ message: "Teammate deleted successfully", type: "success" });
      await loadTeammates();
    } catch (err) {
      console.error("Failed to delete teammate:", err);
      setToast({ message: "Failed to delete teammate", type: "error" });
    } finally {
      setIsDeleting(false);
      setShowDeleteDialog(false);
      setTeammateToDelete(null);
    }
  };

  /**
   * Cancels the delete operation and closes the dialog
   */
  const cancelDeleteTeammate = () => {
    setShowDeleteDialog(false);
    setTeammateToDelete(null);
  };

  const handleEditTeammate = (teammate: Agent) => {
    setSelectedTeammate(teammate);
    setView("edit");
  };

  const handleTeammateCreated = async () => {
    setView("list");
    await loadTeammates();
    setToast({ message: "Teammate created successfully", type: "success" });
  };

  const handleTeammateUpdated = async () => {
    setView("list");
    await loadTeammates();
    setToast({ message: "Teammate updated successfully", type: "success" });
  };

  const renderIcon = (iconName: string) => {
    const Icon = TEAMMATE_ICONS[iconName as TeammateIconName] || TEAMMATE_ICONS.users;
    return <Icon className="h-12 w-12" />;
  };

  // Handle Create/Edit view
  if (view === "create") {
    return (
      <CreateAgent
        onBack={() => setView("list")}
        onAgentCreated={handleTeammateCreated}
        defaultRoleType="teamlead"
      />
    );
  }

  if (view === "edit" && selectedTeammate) {
    return (
      <CreateAgent
        agent={selectedTeammate}
        onBack={() => setView("list")}
        onAgentCreated={handleTeammateUpdated}
        defaultRoleType="teamlead"
      />
    );
  }

  return (
    <div className={cn("flex flex-col h-full bg-background", className)}>
      <div className="w-full max-w-6xl mx-auto flex flex-col h-full p-6">
        {/* Header */}
        <motion.div
          initial={{ opacity: 0, y: -20 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.3 }}
          className="mb-6"
        >
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <Button
                variant="ghost"
                size="icon"
                onClick={onBack}
                className="h-8 w-8"
              >
                <ArrowLeft className="h-4 w-4" />
              </Button>
              <div>
                <h1 className="text-heading-1">Members</h1>
                <p className="mt-1 text-body-small text-muted-foreground">
                  Manage all agents and team members
                </p>
              </div>
            </div>
            <Button
              onClick={() => setView("create")}
              size="default"
              className="flex items-center gap-2"
            >
              <Plus className="h-4 w-4" />
              Add Member
            </Button>
          </div>
        </motion.div>

        {/* Error display */}
        {error && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            className="mb-4 rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-body-small text-destructive">
            {error}
          </motion.div>
        )}

        {/* Main Content */}
        <div className="flex-1 overflow-y-auto">
          <AnimatePresence mode="wait">
            <motion.div
              key="teammates"
              initial={{ opacity: 0, y: 20 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -20 }}
              transition={{ duration: 0.2 }}
              className="pt-6"
            >
              {loading ? (
                <div className="flex items-center justify-center h-64">
                  <Loader2 className="w-8 h-8 animate-spin text-muted-foreground" />
                </div>
              ) : teammates.length === 0 ? (
                <div className="flex flex-col items-center justify-center h-64 text-center">
                  <Users className="h-16 w-16 text-muted-foreground mb-4" />
                  <h3 className="text-heading-4 mb-2">No members yet</h3>
                  <p className="text-body-small text-muted-foreground mb-4">
                    Add your first team member to get started
                  </p>
                  <Button onClick={() => setView("create")} size="default">
                    <Plus className="h-4 w-4 mr-2" />
                    Add Member
                  </Button>
                </div>
              ) : (
                <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
                  <AnimatePresence mode="popLayout">
                    {teammates.map((teammate, index) => (
                      <motion.div
                        key={teammate.id}
                        initial={{ opacity: 0, scale: 0.9 }}
                        animate={{ opacity: 1, scale: 1 }}
                        exit={{ opacity: 0, scale: 0.9 }}
                        transition={{ duration: 0.2, delay: index * 0.05 }}
                      >
                        <Card className="h-full hover:shadow-lg transition-shadow">
                          <CardContent className="p-6 flex flex-col items-center text-center">
                            <div className="mb-4 p-4 rounded-full bg-primary/10 text-primary">
                              {renderIcon(teammate.icon)}
                            </div>
                            <h3 className="text-heading-4 mb-1">
                              {teammate.name}
                            </h3>
                            <div className="flex items-center gap-2 mb-2">
                              <Badge variant="secondary" className="text-xs">
                                {teammate.role_type || 'general'}
                              </Badge>
                            </div>
                            {teammate.nickname && (
                              <p className="text-caption text-muted-foreground mb-2">
                                @{teammate.nickname}
                              </p>
                            )}
                            <p className="text-caption text-muted-foreground">
                              Created: {new Date(teammate.created_at).toLocaleDateString()}
                            </p>
                            {teammate.default_task && (
                              <p className="text-caption text-muted-foreground mt-2 line-clamp-2">
                                {teammate.default_task}
                              </p>
                            )}
                          </CardContent>
                          <CardFooter className="p-4 pt-0 flex justify-center gap-1 flex-wrap">
                            <DropdownMenu>
                              <DropdownMenuTrigger asChild>
                                <Button
                                  size="sm"
                                  variant="ghost"
                                  className="flex items-center gap-1"
                                >
                                  <ChevronDown className="h-3 w-3" />
                                  Actions
                                </Button>
                              </DropdownMenuTrigger>
                              <DropdownMenuContent align="end">
                                <DropdownMenuItem onClick={() => handleEditTeammate(teammate)}>
                                  <Edit className="h-4 w-4 mr-2" />
                                  Edit
                                </DropdownMenuItem>
                                <DropdownMenuItem
                                  onClick={() => handleDeleteTeammate(teammate)}
                                  className="text-destructive"
                                >
                                  <Trash2 className="h-4 w-4 mr-2" />
                                  Delete
                                </DropdownMenuItem>
                              </DropdownMenuContent>
                            </DropdownMenu>
                          </CardFooter>
                        </Card>
                      </motion.div>
                    ))}
                  </AnimatePresence>
                </div>
              )}
            </motion.div>
          </AnimatePresence>
        </div>
      </div>

      {/* Toast Notification */}
      <ToastContainer>
        {toast && (
          <Toast
            message={toast.message}
            type={toast.type}
            onDismiss={() => setToast(null)}
          />
        )}
      </ToastContainer>

      {/* Delete Confirmation Dialog */}
      <Dialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Trash2 className="h-5 w-5 text-destructive" />
              Delete Team Member
            </DialogTitle>
            <DialogDescription>
              Are you sure you want to delete "{teammateToDelete?.name}"?
              This action cannot be undone and will permanently remove this team member.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter className="flex flex-col-reverse sm:flex-row sm:justify-end gap-2">
            <Button
              variant="outline"
              onClick={cancelDeleteTeammate}
              disabled={isDeleting}
              className="w-full sm:w-auto"
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={confirmDeleteTeammate}
              disabled={isDeleting}
              className="w-full sm:w-auto"
            >
              {isDeleting ? (
                <>
                  <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-white mr-2" />
                  Deleting...
                </>
              ) : (
                <>
                  <Trash2 className="h-4 w-4 mr-2" />
                  Delete
                </>
              )}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
};

export default Teammates;
