(ns clojure.string-test.replace-first
  (:require [clojure.string :as str]
            [clojure.test :as t :refer [deftest testing is]]))

(deftest test-replace-first-string-match
  (is (= "baa" (str/replace-first "aaa" "a" "b")))
  (is (= "hello" (str/replace-first "hello" "x" "y"))))

(deftest test-replace-first-pattern-match
  (is (= "host" (str/replace-first "--host" #"^--" "")))
  (is (= "baa" (str/replace-first "aaa" #"a" "b")))
  (is (= "hello world  end" (str/replace-first "hello  world  end" #"\s+" " "))))
