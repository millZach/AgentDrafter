;; The judge's pass over one drawing: record that it opened, write AutoCAD's ASCII DXF (2018 format,
;; 16 decimal places), then AUDIT without fixing. Writes <id>.status and <id>.dxf next to the drawing;
;; the audit report goes to the console log. Nothing in the drawing is saved.
;; The judge pastes these forms into each script, because SECURELOAD blocks (load) from the scratch folder.
(defun adb-st (pre base line / f)
  (setq f (open (strcat pre base ".status") "a"))
  (write-line line f)
  (close f))
(defun adb-judge (mode / nm base pre)
  (setq nm (getvar "DWGNAME") base (substr nm 1 (- (strlen nm) 4)) pre (getvar "DWGPREFIX"))
  (adb-st pre base (strcat "DWGNAME\t" nm))
  (adb-st pre base (strcat "MODE\t" mode))
  (command "_.DXFOUT" (strcat pre base ".dxf") "_V" "2018" "16")
  (while (= 1 (logand 1 (getvar "CMDACTIVE"))) (command ""))
  (adb-st pre base "DXFOUT_DONE\t1")
  (command "_.AUDIT" "_N")
  (while (= 1 (logand 1 (getvar "CMDACTIVE"))) (command ""))
  (adb-st pre base "AUDIT_DONE\t1")
  (princ))
