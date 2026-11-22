# Spec: Kanban pull request actions: unlink, add by name, and pull requests not matched to a task

**Status:** active. Built in Kanban #48257.
**Date:** 2026-11-21
**Author:** Camper@narko
**Changes:** the Kanban drawer's pull request rows and the Kanban board menu.

## 1. Requirement

A task's card knows its pull requests in every repository. The person must be able to correct that link by hand, and to see pull requests that no task claimed.

## 2. What the person can do

- **Unlink.** Each pull request in a task's drawer has an Unlink control. It takes the pull request off that task.
- **Add by name.** The drawer has a field that takes a pull request as `owner/repo#number` or a pull request address, and a button to link it to the task. If the pull request is already on the task the drawer says so and changes nothing.
- **Not followed.** A pull request in a repository AgentMux cannot read yet shows a "Not followed" pill with the reason, so a missing state is not mistaken for an open one.
- **Pull requests not matched to a task.** The board menu has an entry that opens a list of the last 50 pull requests that matched no task. Each row shows the repository, the number and its state, with a field for a task number and a "Link to task" action, and a Dismiss action that removes the row without linking.

## 3. How it is done

- The actions go through the Kanban's `act` command: `pr_link`, `pr_unlink`, `pr_unmatched` and `pr_dismiss`. The result says whether anything changed and carries the unmatched list.
- Every visible text is in the dictionary; the controls are the shared kit's.
- The panel works in a pane as narrow as the Kanban's own minimum.

## 4. Not in this spec

Connecting an account so a private repository can be followed is a separate change.
