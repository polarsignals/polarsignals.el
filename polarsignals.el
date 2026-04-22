;;; polarsignals.el  -*- lexical-binding:t -*-

(provide 'polarsignals)
(require 'polarsignals-module)

(defcustom polarsignals/token-file (locate-user-emacs-file "polarsignals.plstore")
  "File path where Polar Signals OAuth tokens are stored."
  :group 'polarsignals
  :type 'file)

(defcustom polarsignals/default-project-id nil
  "Default project ID"
  :group 'polarsignals
  :type '(choice (const nil)
                 string))

(defcustom polarsignals/default-source-rewrite-rules nil
  ;; todo - better docs
  ;; todo - validate regex
  "Pairs of regex and replacement string for mapping source files"
  :group 'polarsignals
  :type '(repeat
          (list string string)))

(defconst polarsignals/cpu-time-query "parca_agent:samples:count:cpu:nanoseconds:delta")

(defconst polarsignals/auth-refresh-buffer (* 60 10))

(defcustom polarsignals/default-query polarsignals/cpu-time-query
  "The default query for Polar Signals"
  :group 'polarsignals
  :type 'string)

(defcustom polarsignals/default-query-time-range `(relative ,(* 15 60))
  "The default time range for Polar Signals queries"
  :group 'polarsignals
  :type '(choice
          (list :tag "Time since the present moment"
                (const :format "" relative)
                (number :tag "Seconds"))
          (list :tag "Absolute time range"
                (const :format "" absolute)
                (number :tag "Begin (in Unix time, i.e. seconds since the epoch)")
                (number :tag "End (in Unix time, i.e. seconds since the epoch)"))))

(defvar polarsignals/open-store nil)

(defcustom polarsignals/target-foreground-color "red"
  "The color of the annotation for 100%"
  :group 'polarsignals
  :type 'color)

;; TODO - understand this, claude wrote it
(defun polarsignals/blended-color (percent)
  (cl-flet ((lerp (a b) (+ a (* (- b a) (/ percent 100.0))))
            (rgb-to-hsv (r g b)
              (let* ((cmax (max r g b))
                     (cmin (min r g b))
                     (delta (- cmax cmin))
                     (h (cond ((= delta 0.0) 0.0)
                              ((= cmax r) (* 60.0 (mod (/ (- g b) delta) 6.0)))
                              ((= cmax g) (* 60.0 (+ (/ (- b r) delta) 2.0)))
                              (t          (* 60.0 (+ (/ (- r g) delta) 4.0)))))
                     (s (if (= cmax 0.0) 0.0 (/ delta cmax)))
                     (v cmax))
                (list h s v)))
            (hsv-to-rgb (h s v)
              (let* ((c (* v s))
                     (x (* c (- 1.0 (abs (- (mod (/ h 60.0) 2.0) 1.0)))))
                     (m (- v c))
                     (rgb1 (cond ((< h 60.0)  (list c x 0.0))
                                 ((< h 120.0) (list x c 0.0))
                                 ((< h 180.0) (list 0.0 c x))
                                 ((< h 240.0) (list 0.0 x c))
                                 ((< h 300.0) (list x 0.0 c))
                                 (t           (list c 0.0 x)))))
                (mapcar (lambda (comp) (+ comp m)) rgb1))))
    (-let* (((r g b) (color-values (face-foreground 'default)))
            ((tr tg tb) (color-values polarsignals/target-foreground-color))
            ;; color-values returns 0-65535, normalize to 0.0-1.0
            ((h1 s1 v1) (rgb-to-hsv (/ r 65535.0) (/ g 65535.0) (/ b 65535.0)))
            ((h2 s2 v2) (rgb-to-hsv (/ tr 65535.0) (/ tg 65535.0) (/ tb 65535.0)))
            ;; take the shortest path around the hue circle
            (hdiff (- h2 h1))
            (hdiff (cond ((> hdiff 180.0) (- hdiff 360.0))
                         ((< hdiff -180.0) (+ hdiff 360.0))
                         (t hdiff)))
            (bh (mod (+ h1 (* hdiff (/ percent 100.0))) 360.0))
            (bs (lerp s1 s2))
            (bv (lerp v1 v2))
            ((br bg bb) (hsv-to-rgb bh bs bv)))
      (format "#%02x%02x%02x" (* br 255.0) (* bg 255.0) (* bb 255.0)))))


;; TODO - should we close it after a while?
(defun polarsignals/ensure-plstore ()
  (let ((plstore (or polarsignals/open-store
                     (plstore-open polarsignals/token-file))))
     (setq polarsignals/open-store plstore)
     plstore))

(defun polarsignals/do-auth
    (project-id)
  (let* ((pending (polarsignals-module-auth-begin))
         (url (polarsignals-module-auth-pending-url pending)))
    (browse-url url)
    (let* ((code (read-string "code? "))
           (result (polarsignals-module-auth-resume pending code)))
      result)))


;; TODO - why does edebug choke on this?
;; (defun x
;;     ()
;;   (or
;;    (())))

;; return access token
;; todo - it'd be nicer to use the redirect-to-localhost
;; trick to get the code automatically. This could be done on the
;; Rust side; maybe associate an HTTP server with the "pending request" ?
;; (and kill it after a few minutes to avoid wasting a port forever?)
(defun polarsignals/auth-for-project-and-store
    (project-id)
   (let* ((plstore (polarsignals/ensure-plstore))
         (possibly-stale-token-data
          (or
           (cdr (plstore-get plstore project-id))
           (polarsignals/do-auth project-id)))
         (vunt (plist-get possibly-stale-token-data :valid-until))
         (must-refresh (>= (+ (float-time) polarsignals/auth-refresh-buffer) vunt))
         (token-data
          (if (not must-refresh)
              possibly-stale-token-data
            (condition-case e
                (polarsignals-module-auth-begin-refresh (plist-get possibly-stale-token-data :refresh))
              (error
               (message "couldn't refresh: %s" e)
               (polarsignals/do-auth project-id))))))
    (when token-data
      (plstore-put plstore project-id `(:project-id ,project-id) token-data)
      (plstore-save plstore)
      (plist-get token-data :access))))

(defun polarsignals/query-data-for-file (project-id filename query start end)
  (let* ((access (polarsignals/auth-for-project-and-store project-id))
         (all-data
          (polarsignals-module-source-query access filename "" project-id query start end)))
    (alist-get filename all-data nil nil #'string-equal)))


(defun polarsignals/set-margin-width (w)
  (let ((buf (current-buffer)))
    (setq left-margin-width w)    
    (cl-loop
     for win in (get-buffer-window-list buf)
     do (set-window-buffer win buf))))

(defun polarsignals/annotate-buffer (buf data)
  (with-current-buffer buf
    (unless (boundp 'polarsignals/old-margin-width)
      (setq polarsignals/old-margin-width left-margin-width))
    (polarsignals/set-margin-width 7)
    (-let (((sum-cumul sum-flat) (cl-loop
                                  for (_lineno cumul flat) in data
                                  sum cumul into sum-cumul
                                  sum flat into sum-flat
                                  finally return `(,sum-cumul ,sum-flat)))
           all-overlays)
      (cl-loop
       for (lineno cumul flat) in data
       do
       (goto-char (point-min))
       ;; TODO -- error if past the end of the buffer?
       (forward-line (1- lineno))
       (let* ((ov (make-overlay (line-end-position) (line-end-position)))
              (pct-flat (* 100.0 (/ (float flat) sum-flat)))
              (pct-cumul (* 100.0 (/ (float cumul) sum-cumul))))
         (overlay-put ov 'before-string
                      (propertize " " 'display
                                  `((margin left-margin)
                                    ,(concat (propertize (format "%3d " (round pct-cumul))
                                                         'face `(:foreground ,(polarsignals/blended-color pct-cumul)))
                                             (propertize (format "%3d" (round pct-flat))
                                                         'face `(:foreground ,(polarsignals/blended-color pct-flat)))))))
         (push ov all-overlays)))
      (when (boundp 'polarsignals/all-overlays)
        (mapc #'delete-overlay polarsignals/all-overlays))
      (setq-local polarsignals/all-overlays all-overlays))))

(defun polarsignals/clear ()
  (interactive)
  (when (boundp 'polarsignals/all-overlays)
    (mapc #'delete-overlay polarsignals/all-overlays)
    (makunbound 'polarsignals/all-overlays))
  
  (when (boundp 'polarsignals/old-margin-width)
    (polarsignals/set-margin-width polarsignals/old-margin-width)
    (makunbound 'polarsignals/old-margin-width)))


(defun polarsignals/query-for-current-buffer (buffer project-id filename query start end)
  (interactive   
   (let* ((cur-buf-fname (buffer-file-name (current-buffer)))
          (default-filename
           (and cur-buf-fname
                (cl-loop for (regex replacement) in polarsignals/default-source-rewrite-rules
                         when (string-match regex cur-buf-fname)
                         return (replace-match replacement nil nil cur-buf-fname)
                         finally return cur-buf-fname)))
          (use-defaults (not current-prefix-arg))
          (plstore (polarsignals/ensure-plstore))
          (now (float-time)))
     (cl-flet ((mk-startend (custval)
                 (pcase custval
                   (`(relative ,secs)
                    (list (- now secs) now))
                   (`(absolute ,start end)
                    (list start end)))))
       (-cons*
        ;; buffer
        (current-buffer)
        ;; project-id
        (or (and use-defaults polarsignals/default-project-id)
            (completing-read "Project ID: " (mapcar #'car (plstore--get-alist plstore))))
        ;; filename
        ;; Todo - completing-read with reasonable collection for all that follows?
        ;; Or perhaps a history list?
        (or (and use-defaults default-filename)
            (read-string "File path in Polar Signals: " default-filename))
        ;; query
        (or (and use-defaults polarsignals/default-query)
            (read-string "Query: " polarsignals/default-query))
        ;; start/end
        (let ((default-startend (mk-startend polarsignals/default-query-time-range)))
          (if use-defaults
              default-startend
              (list
               (read-number "Query start (Unix time): " (car default-startend))
               (read-number "Query end (Unix time): " (cadr default-startend)))))))))
  (if-let ((data (polarsignals/query-data-for-file project-id filename query start end)))
      (polarsignals/annotate-buffer buffer data)
    (error "no data found")))
