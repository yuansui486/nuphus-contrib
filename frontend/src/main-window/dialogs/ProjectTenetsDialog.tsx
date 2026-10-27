import { IconShield } from '../../ui/Icons'
import { useLanguage } from '../../locales'
import { CompactModal } from '../layout/CompactModal'
import { ProjectTenetsPanel } from './ProjectTenetsPanel'
import type { TenetProjectScope } from './tenetScope'

interface ProjectTenetsDialogProps {
  project: TenetProjectScope | null
  onClose: () => void
}

/** 项目原则独立弹窗（会话工作台组头入口） */
export function ProjectTenetsDialog({ project, onClose }: ProjectTenetsDialogProps) {
  const { t } = useLanguage()
  return (
    <CompactModal
      open={!!project}
      onClose={onClose}
      title={
        project
          ? t('memory.tenet.projectTitleWithName', project.name)
          : t('memory.tenet.projectTitle')
      }
      icon={<IconShield size={14} />}
      size="sm"
      className="compact-modal--fit pt-modal"
    >
      {project && <ProjectTenetsPanel project={project} />}
    </CompactModal>
  )
}
