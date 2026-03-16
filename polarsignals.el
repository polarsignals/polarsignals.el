;;; polarsignals.el  -*- lexical-binding:t -*-

(provide 'polarsignals)
(require 'polarsignals-module)

(defcustom polarsignals/token-file (locate-user-emacs-file "polarsignals.plstore")
  "File path where Polar Signals OAuth tokens are stored."
  :group 'polarsignals
  :type 'file)

(defvar polarsignals/open-store nil)

;; TODO - should we close it after a while?
(defun polarsignals/ensure-plstore ()
  (let ((plstore (or polarsignals/open-store
                     (plstore-open polarsignals/token-file))))
     (setq polarsignals/open-store plstore)
     plstore))

;; return access token
;; todo - handle refresh
;; todo - it'd be nicer to use the redirect-to-localhost
;; trick to get the code automatically. This could be done on the
;; Rust side; maybe associate an HTTP server with the "pending request" ?
;; (and kill it after a few minutes to avoid wasting a port forever?)
(defun polarsignals/auth-for-project-and-store
    (project-id)
  (let* ((plstore (polarsignals/ensure-plstore))
         (maybe-token-data (cdr (plstore-get plstore project-id)))
         (token-data (or maybe-token-data
                         (let* ((pending (polarsignals-module-auth-begin))
                                (url (polarsignals-module-auth-pending-url pending)))
                           (browse-url url)
                           (let* ((code (read-string "code? "))
                                  (result (polarsignals-module-auth-resume pending code)))
                             (plstore-put plstore project-id `(:project-id ,project-id) result)
                             (plstore-save plstore)
                             result)))))
    (plist-get token-data :access)))
